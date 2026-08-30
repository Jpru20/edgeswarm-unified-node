use crate::core::{
    auth_client::SupabaseAuthClient,
    backend_client::DEFAULT_BACKEND_URL,
    deterministic_executor,
    model_discovery::discover_models,
    model_provisioning::{
        fetch_model_recommendation_v1,
        provision_recommendation_v1,
        set_model_download_stage_v1,
    },
    real_capacity_certification::certify_model_path_v1,
    production_heartbeat::ProductionHeartbeatV1,
    production_inference::ProductionLlamaClient,
    production_task_http::{
        poll_once, read_auth, send_heartbeat, send_stream_frame_with_retry, submit_with_retry,
    },
    result_signing,
    task_client::{build_submit_result, GetJobsResponse, TaskEnvelope},
    wallet_account::DeviceWallet,
    wallet_client::WorkerWalletClient,
    wallet_identity::{select_wallet_row, WalletRowDecision},
    wallet_public_identity::WalletPublicIdentity,
    wallet_vault, NodeState,
};
use crate::runtime::llama_process::{
    resolve_llama_server_path_v1, resolve_model_root_v1, LlamaProcessConfig,
    ManagedLlamaProcess,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
use sysinfo::Disks;
use zeroize::{Zeroize, Zeroizing};

static NODE_SERVICE_LOGS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn node_service_log_buffer() -> &'static Mutex<Vec<String>> {
    NODE_SERVICE_LOGS.get_or_init(|| Mutex::new(Vec::new()))
}

fn push_node_service_log(line: String) {
    if let Ok(mut logs) = node_service_log_buffer().lock() {
        logs.push(line);

        if logs.len() > 200 {
            let excess = logs.len() - 200;
            logs.drain(0..excess);
        }
    }
}

pub fn clear_node_service_logs() {
    if let Ok(mut logs) = node_service_log_buffer().lock() {
        logs.clear();
    }
}

pub fn node_service_logs() -> Vec<String> {
    node_service_log_buffer()
        .lock()
        .map(|logs| logs.clone())
        .unwrap_or_default()
}

macro_rules! println {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        std::println!("{}", line);
        push_node_service_log(line);
    }};
}

fn first_task(mut r: GetJobsResponse) -> Option<TaskEnvelope> {
    if !r.tasks.is_empty() {
        Some(r.tasks.remove(0))
    } else {
        r.task
    }
}

fn resolve_active_model_path_v1(selected_model: &str) -> Result<String, String> {
    let root = resolve_model_root_v1()?;

    let mut matches = discover_models(&root)
        .into_iter()
        .filter(|model| model.selected_model == selected_model)
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| left.path.cmp(&right.path));

    let model = matches
        .into_iter()
        .next()
        .ok_or_else(|| format!("certified_model_artifact_missing:{selected_model}"))?;

    Ok(model.path.to_string_lossy().to_string())
}

fn execution_acceleration_v1() -> String {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        let layers = env::var("EDGESWARM_LLAMA_GPU_LAYERS")
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(999);

        if layers > 0 {
            return "metal".into();
        }
    }

    env::var("EDGESWARM_EXECUTION_ACCELERATION")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "cpu".into())
}

fn failure_payload(
    task: &TaskEnvelope,
    email: &str,
    worker: &str,
    hardware: &str,
    private_key: &str,
    reason: &str,
) -> Result<Value, String> {
    let output = json!({
        "error": reason
    })
    .to_string();

    let hash = Sha256::digest(output.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    let signature =
        result_signing::sign_result(&task.task_id_text(), 0, &hash, hardware, private_key)?;

    Ok(json!({
        "fileHash": hash,
        "payload": {
            "taskId": task.task_id,
            "worker": worker,
            "providerEmail": email,
            "score": 0,
            "signature": signature,
            "hardwareId": hardware,
            "aiOutput": output,
            "status": "error",
            "latency_ms": 0,
            "requiredModel":
                task.required_model.clone().unwrap_or_default(),
            "modelIdUsed": task.selected_model.clone().unwrap_or_else(|| "unavailable".into()),
            "runtime": "llama.cpp",
            "runtimeAcceleration": "unknown"
        }
    }))
}

fn build_task_submit_payload(
    task: &TaskEnvelope,
    llama: Option<&ProductionLlamaClient>,
    provider_email: &str,
    worker: &str,
    hardware: &str,
    private_key: &str,
    active_selected_model: Option<&str>,
    active_capability: Option<&str>,
    active_runtime: Option<&str>,
    runtime_acceleration: &str,
    stream_neural: bool,
    on_chunk: Option<&mut dyn FnMut(&str)>,
) -> Result<Value, String> {
    if let Some(result) = deterministic_executor::execute(task) {
        println!("DETERMINISTIC_EXECUTION_SUCCEEDED=true");
        println!("DETERMINISTIC_MODEL_ID={}", result.model_id_used);

        return serde_json::to_value(build_submit_result(
            task,
            &result.ai_output,
            provider_email,
            worker,
            hardware,
            private_key,
            result.latency_ms,
            result.model_id_used,
            "deterministic",
            "cpu",
        )?)
        .map_err(|_| "deterministic_result_payload_encode_failed".to_string());
    }

    let required = task.required_model.as_deref().unwrap_or("");
    let selected = task.selected_model.as_deref().unwrap_or("");

    let neural_supported = match (active_selected_model, active_capability) {
        (Some(active_model), Some(active_capability)) => {
            required == active_capability
                && (selected.is_empty() || selected == "tier:auto" || selected == active_model)
        }
        _ => false,
    };

    if !neural_supported {
        println!("TASK_SUPPORTED=false");
        return failure_payload(
            task,
            provider_email,
            worker,
            hardware,
            private_key,
            "unsupported_claimed_task",
        );
    }

    let llama = llama.ok_or_else(|| "neural_runtime_unavailable".to_string())?;

    let inference_result = if stream_neural {
        if let Some(callback) = on_chunk {
            llama.execute_streaming(&task.prompt, task.max_output_tokens, |chunk| {
                callback(chunk)
            })
        } else {
            llama.execute(&task.prompt, task.max_output_tokens)
        }
    } else {
        llama.execute(&task.prompt, task.max_output_tokens)
    };

    match inference_result {
        Ok(result) => {
            println!("INFERENCE_SUCCEEDED=true");
            println!("INFERENCE_LATENCY_MS={}", result.latency_ms);

            serde_json::to_value(build_submit_result(
                task,
                &result.ai_output,
                provider_email,
                worker,
                hardware,
                private_key,
                result.latency_ms,
                active_selected_model.ok_or_else(|| "active_model_missing".to_string())?,
                active_runtime.ok_or_else(|| "active_runtime_missing".to_string())?,
                runtime_acceleration,
            )?)
            .map_err(|_| "result_payload_encode_failed".to_string())
        }
        Err(_) => {
            println!("INFERENCE_SUCCEEDED=false");
            failure_payload(
                task,
                provider_email,
                worker,
                hardware,
                private_key,
                "neural_inference_failed",
            )
        }
    }
}

// REALTIME_NEURAL_STREAM_CONTRACT_V1
fn task_realtime_neural_streaming_v1(task: &TaskEnvelope) -> bool {
    let neural = task
        .required_model
        .as_deref()
        .map(|value| value.starts_with("Neural-Inference"))
        .unwrap_or(false);

    if !neural {
        return false;
    }

    task.streaming_contract
        .as_ref()
        .and_then(|contract| contract.effective_mode.as_deref())
        .map(|mode| mode.eq_ignore_ascii_case("realtime"))
        .unwrap_or(false)
}

// TRANSIENT_NODE_TRANSPORT_RESILIENCE_V1
// Temporary infrastructure failures must not tear down an otherwise healthy
// node/runtime. Authentication, update, trust, and malformed-contract errors
// remain fail-closed.
fn transient_node_transport_error_v1(error: &str) -> bool {
    if matches!(
        error,
        "task_poll_network_failed" | "heartbeat_network_failed"
    ) {
        return true;
    }

    for prefix in ["task_poll_http_", "heartbeat_http_"] {
        if let Some(raw) = error.strip_prefix(prefix) {
            if let Ok(status) = raw.parse::<u16>() {
                return matches!(status, 408 | 425 | 429) || (500..=599).contains(&status);
            }
        }
    }

    false
}

#[cfg(test)]
#[test]
fn transient_node_transport_errors_are_classified_v1() {
    assert!(transient_node_transport_error_v1(
        "task_poll_network_failed"
    ));
    assert!(transient_node_transport_error_v1("task_poll_http_502"));
    assert!(transient_node_transport_error_v1("heartbeat_http_503"));
    assert!(transient_node_transport_error_v1("task_poll_http_429"));

    assert!(!transient_node_transport_error_v1("task_poll_http_401"));
    assert!(!transient_node_transport_error_v1("task_poll_http_403"));
    assert!(!transient_node_transport_error_v1(
        "node_update_required_http_426"
    ));
    assert!(!transient_node_transport_error_v1(
        "task_poll_response_invalid"
    ));
}

#[derive(Debug)]
struct NodeServiceInstanceLockV1 {
    _file: std::fs::File,
}

fn acquire_node_service_instance_lock_at_v1(
    data_dir: &std::path::Path,
) -> Result<NodeServiceInstanceLockV1, String> {
    use fs2::FileExt;

    std::fs::create_dir_all(data_dir)
        .map_err(|_| "node_service_lock_directory_failed".to_string())?;

    let lock_path = data_dir.join("node-service.lock");

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|_| "node_service_lock_open_failed".to_string())?;

    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(NodeServiceInstanceLockV1 { _file: file }),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            Err("node_service_already_running".to_string())
        }
        Err(_) => Err("node_service_lock_failed".to_string()),
    }
}

fn acquire_node_service_instance_lock_v1() -> Result<NodeServiceInstanceLockV1, String> {
    acquire_node_service_instance_lock_at_v1(&crate::adapters::app_data_dir())
}

#[cfg(test)]
#[test]
fn node_service_instance_lock_v1_rejects_second_holder() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);

    let dir = std::env::temp_dir().join(format!(
        "edgeswarm-node-lock-test-{}-{}",
        std::process::id(),
        nonce
    ));

    let first = acquire_node_service_instance_lock_at_v1(&dir).expect("first lock should succeed");

    let second = acquire_node_service_instance_lock_at_v1(&dir);

    assert_eq!(
        second.err().as_deref(),
        Some("node_service_already_running")
    );

    drop(first);

    let third = acquire_node_service_instance_lock_at_v1(&dir);

    assert!(third.is_ok(), "lock should release when first holder exits");

    drop(third);
    let _ = std::fs::remove_dir_all(&dir);
}

fn model_disk_free_gb_v1() -> u64 {
    let root = crate::core::model_provisioning::model_root_v1();
    let disks = Disks::new_with_refreshed_list();

    let matched = disks.list().iter()
        .filter(|disk| root.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count());

    matched
        .map(|disk| disk.available_space() / 1024 / 1024 / 1024)
        .or_else(|| {
            disks.list().iter()
                .map(|disk| disk.available_space() / 1024 / 1024 / 1024)
                .max()
        })
        .unwrap_or(0)
}

fn model_recommendation_payload_v1(state: &NodeState) -> Value {
    let ram_gb = state.hardware.total_memory_bytes as f64
        / 1024.0 / 1024.0 / 1024.0;
    let backend = state.acceleration.backend.to_ascii_lowercase();
    let macos = state.platform.os.eq_ignore_ascii_case("macos");

    json!({
        "nodeType": "laptop-node",
        "platform": state.platform.os,
        "architecture": state.platform.architecture,
        "ramGb": ram_gb,
        "diskFreeGb": model_disk_free_gb_v1(),
        "cpuCores": state.hardware.logical_cpu_count,
        "gpuVendor": if macos { "Apple" } else { "" },
        "gpuName": state.acceleration.device_name.clone()
            .unwrap_or_else(|| state.hardware.cpu_brand.clone()),
        "cudaAvailable": backend.contains("cuda"),
        "metalAvailable": backend.contains("metal") || macos
    })
}

fn provision_fresh_model_v1(state: &NodeState) -> Result<bool, String> {
    let payload = model_recommendation_payload_v1(state);
    let base_url = env::var("GCP_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_BACKEND_URL.to_string());

    println!("MODEL_RECOMMENDATION_REQUESTED=true");
    let recommendation =
        fetch_model_recommendation_v1(&base_url, &payload)?;

    println!("MODEL_RECOMMENDATION={}", recommendation.model_id);

    if recommendation.files.is_empty() {
        println!("MODEL_PROVISIONING_REQUIRED=false");
        return Ok(false);
    }

    println!("MODEL_PROVISIONING_REQUIRED=true");
    println!("MODEL_PROVISIONING_STARTED=true");
    let paths = provision_recommendation_v1(&recommendation)?;
    println!("MODEL_PROVISIONING_COMPLETE=true");

    let model_path = paths.first()
        .ok_or_else(|| "provisioned_model_path_missing".to_string())?;
    let runtime_path = resolve_llama_server_path_v1()?;

    println!("MODEL_CERTIFICATION_STARTED=true");
    set_model_download_stage_v1("certifying");
    certify_model_path_v1(
        model_path.to_string_lossy().to_string(),
        runtime_path.to_string_lossy().to_string(),
    )?;
    println!("MODEL_CERTIFICATION_COMPLETE=true");
    set_model_download_stage_v1("ready");

    Ok(true)
}

pub fn run_node_service(
    stop: Arc<AtomicBool>,
    mut wallet_password: Zeroizing<String>,
) -> Result<(), String> {
    let auth_client = SupabaseAuthClient::from_env()?;
    auth_client.ensure_valid_session(true)?;

    let mut auth = read_auth()?;
    let mut state = NodeState::detect();
    let hardware = state.hardware_identity.hardware_id.clone();

    let public_wallet = WalletPublicIdentity::load_default()?;

    if public_wallet.hardware_id != hardware {
        return Err("wallet_hardware_mismatch".into());
    }

    let heartbeat_only = env::var("EDGESWARM_HEARTBEAT_ONLY")
        .map(|value| value.trim() == "1")
        .unwrap_or(false);

    // Heartbeat-only mode advertises exactly what NodeState has
    // actually validated/certified, then exits before wallet unlock,
    // runtime startup, or /get-jobs.
    if heartbeat_only {
        let heartbeat = ProductionHeartbeatV1::from_node_state(
            &state,
            env!("CARGO_PKG_VERSION"),
            "laptop",
            &[],
        );

        let capability_mode = if heartbeat.eligible_model_capabilities.is_empty() {
            "deterministic_only"
        } else {
            "neural_ready"
        };

        let http = Client::builder()
            .timeout(Duration::from_secs(65))
            .build()
            .map_err(|_| "backend_http_client_failed".to_string())?;

        let status = send_heartbeat(&http, &auth_client, &mut auth, &heartbeat)?;

        println!("READINESS_HEARTBEAT_HTTP_STATUS={status}");
        println!("HEARTBEAT_ONLY_MODE=true");
        println!("NODE_CAPABILITY_MODE={capability_mode}");

        if let Some(model) = heartbeat.model_id.as_deref() {
            println!("ADVERTISED_MODEL={model}");
        }

        if let Some(capability) = heartbeat.model_capability.as_deref() {
            println!("ADVERTISED_MODEL_CAPABILITY={capability}");
        }

        println!("ADVERTISED_CONCURRENCY={}", heartbeat.concurrency_limit);
        println!("WALLET_UNLOCKED=false");
        println!("NEURAL_RUNTIME_STARTED=false");
        println!("HEARTBEAT_SENT=true");
        println!("GET_JOBS_CALLED=false");
        println!("TASK_CLAIMED=false");
        println!("RESULT_SUBMITTED=false");

        return Ok(());
    }

    let _node_service_instance_lock = acquire_node_service_instance_lock_v1()?;

    println!("NODE_SERVICE_INSTANCE_LOCK_ACQUIRED=true");

    let wallet_client = WorkerWalletClient::from_env()?;
    let rows = wallet_client.rows_for_email(&auth.access_token, &auth.provider_email)?;

    let row_index = match select_wallet_row(&rows, &hardware)? {
        WalletRowDecision::ExactDevice { row_index } => row_index,
        WalletRowDecision::ClaimLegacy { .. } => return Err("runner_refuses_legacy_wallet".into()),
        WalletRowDecision::CreateDevice => return Err("runner_refuses_wallet_creation".into()),
    };

    let private_key = Zeroizing::new(wallet_vault::decrypt(
        &rows[row_index].private_key,
        wallet_password.as_str(),
        &auth.provider_email,
    )?);

    // Wallet unlock material is no longer needed once the device
    // private key has been decrypted. Remove this service copy now.
    wallet_password.zeroize();

    let recovered = DeviceWallet::from_private_key(private_key.as_str())?;

    if !recovered
        .wallet_address()
        .eq_ignore_ascii_case(&public_wallet.wallet_address)
    {
        return Err("wallet_unlock_identity_mismatch".into());
    }

    println!("WALLET_UNLOCKED=true");

    let mut base_heartbeat =
        ProductionHeartbeatV1::from_node_state(&state, env!("CARGO_PKG_VERSION"), "laptop", &[]);

    if base_heartbeat.model_id.is_none()
        && provision_fresh_model_v1(&state)?
    {
        state = NodeState::detect();
        base_heartbeat = ProductionHeartbeatV1::from_node_state(
            &state,
            env!("CARGO_PKG_VERSION"),
            "laptop",
            &[],
        );
        println!("MODEL_STATE_REFRESHED=true");
    }

    if base_heartbeat.concurrency_limit != 1 {
        return Err("runner_capacity_gate_failed".into());
    }

    let active_selected_model = base_heartbeat.model_id.clone();

    let active_capability = base_heartbeat.model_capability.clone();

    let active_runtime = base_heartbeat.runtime.clone();

    let runtime_acceleration = execution_acceleration_v1();

    let neural_ready = active_selected_model.is_some()
        && active_capability
            .as_deref()
            .map(|capability| capability.starts_with("Neural-Inference"))
            .unwrap_or(false);

    let deterministic_ready = ["Exact-Extraction", "Data-Scraper", "Distributed-Compute"]
        .iter()
        .all(|required| {
            base_heartbeat
                .capabilities
                .iter()
                .any(|capability| capability == required)
        });

    if !neural_ready && !deterministic_ready {
        return Err("runner_no_executable_capabilities".into());
    }

    let poll_capabilities = base_heartbeat.capabilities.clone();

    let mut _managed_llama: Option<ManagedLlamaProcess> = None;

    let llama = if neural_ready {
        let llama_base_url = match env::var("EDGESWARM_LLAMA_BASE_URL") {
            Ok(value) if !value.trim().is_empty() => {
                println!("LLAMA_RUNTIME_OWNERSHIP=external");
                value
            }
            _ => {
                let selected_model = active_selected_model
                    .as_deref()
                    .ok_or_else(|| "active_model_missing".to_string())?;

                let model_path = resolve_active_model_path_v1(selected_model)?;

                println!("ACTIVE_EXECUTION_MODEL={selected_model}");

                let config = LlamaProcessConfig::for_model(model_path)?;
                let runtime = ManagedLlamaProcess::start(&config)?;
                let base_url = runtime.base_url().to_string();

                println!("LLAMA_RUNTIME_OWNERSHIP=managed");
                _managed_llama = Some(runtime);
                base_url
            }
        };

        let client = ProductionLlamaClient::new(llama_base_url)?;
        client.health_check()?;
        println!("LOCAL_RUNTIME_READY=true");
        Some(client)
    } else {
        println!("NODE_CAPABILITY_MODE=deterministic_only");
        println!("NEURAL_RUNTIME_REQUIRED=false");
        None
    };

    let http = Client::builder()
        .timeout(Duration::from_secs(65))
        .build()
        .map_err(|_| "backend_http_client_failed".to_string())?;

    let readiness_status = send_heartbeat(&http, &auth_client, &mut auth, &base_heartbeat)?;

    println!("READINESS_HEARTBEAT_HTTP_STATUS={readiness_status}");

    if heartbeat_only {
        println!("HEARTBEAT_ONLY_MODE=true");
        println!("HEARTBEAT_SENT=true");
        println!("GET_JOBS_CALLED=false");
        println!("TASK_CLAIMED=false");
        println!("RESULT_SUBMITTED=false");
        return Ok(());
    }

    let mut last_idle_heartbeat = Instant::now();

    loop {
        // Graceful stop boundary: never claim another task after
        // the user requests STOP.
        if stop.load(Ordering::Acquire) {
            println!("NODE_SERVICE_STOP_REQUESTED=true");
            println!("NODE_SERVICE_STOPPED=true");
            return Ok(());
        }

        let poll = match poll_once(
            &http,
            &auth_client,
            &mut auth,
            &hardware,
            &poll_capabilities,
        ) {
            Ok(poll) => poll,

            Err(error) if transient_node_transport_error_v1(&error) => {
                println!("POLL_TRANSIENT_ERROR={error}");
                println!("POLL_RETRYING=true");

                for _ in 0..20 {
                    if stop.load(Ordering::Acquire) {
                        println!("NODE_SERVICE_STOP_REQUESTED=true");
                        println!("NODE_SERVICE_STOPPED=true");
                        return Ok(());
                    }

                    thread::sleep(Duration::from_millis(100));
                }

                continue;
            }

            Err(error) => return Err(error),
        };

        if poll.blocked {
            println!("TASK_CLAIMED=false");
            println!("POLL_BLOCKED=true");
            println!(
                "POLL_BLOCK_REASON={}",
                poll.block_reason.as_deref().unwrap_or("unspecified")
            );
            println!(
                "POLL_BLOCK_MESSAGE={}",
                poll.message.as_deref().unwrap_or("")
            );
            return Ok(());
        }

        let Some(task) = first_task(poll) else {
            if last_idle_heartbeat.elapsed() >= Duration::from_secs(15) {
                match send_heartbeat(&http, &auth_client, &mut auth, &base_heartbeat) {
                    Ok(status) => {
                        println!("IDLE_HEARTBEAT_HTTP_STATUS={status}");
                    }

                    Err(error) if transient_node_transport_error_v1(&error) => {
                        println!("IDLE_HEARTBEAT_TRANSIENT_ERROR={error}");
                        println!("IDLE_HEARTBEAT_RETRYING=true");
                    }

                    Err(error) => return Err(error),
                }

                last_idle_heartbeat = Instant::now();
            }

            // Keep idle STOP response quick without interrupting
            // an active task lifecycle.
            for _ in 0..10 {
                if stop.load(Ordering::Acquire) {
                    println!("NODE_SERVICE_STOP_REQUESTED=true");
                    println!("NODE_SERVICE_STOPPED=true");
                    return Ok(());
                }

                thread::sleep(Duration::from_millis(100));
            }

            continue;
        };

        let task_id = task.task_id_text();

        println!("TASK_CLAIMED=true");
        println!("TASK_ID={task_id}");

        let mut active = ProductionHeartbeatV1::from_node_state(
            &state,
            env!("CARGO_PKG_VERSION"),
            "laptop",
            &[],
        );

        active.current_task_ids = vec![task_id.clone()];

        match send_heartbeat(&http, &auth_client, &mut auth, &active) {
            Ok(status) => println!("ACTIVE_HEARTBEAT_HTTP_STATUS={status}"),

            Err(_) => {
                println!("ACTIVE_HEARTBEAT_FAILED=true");

                let failure = failure_payload(
                    &task,
                    &auth.provider_email,
                    &public_wallet.wallet_address,
                    &hardware,
                    private_key.as_str(),
                    "active_task_heartbeat_failed",
                )?;

                let outcome = submit_with_retry(&http, &auth_client, &mut auth, &failure)?;

                println!("FAILURE_RESULT_HTTP_STATUS={}", outcome.status);
                println!("TASK_LIFECYCLE_STOPPED_AFTER_HEARTBEAT_FAILURE=true");
                return Ok(());
            }
        }

        let provider_email_for_task_v1 = auth.provider_email.clone();

        let stream_requested_v1 = task_realtime_neural_streaming_v1(&task);

        let mut stream_enabled_v1 = false;
        let mut stream_sequence_v1 = 0_u64;
        let mut stream_buffer_v1 = String::new();
        let mut stream_last_flush_v1 = Instant::now();

        // ASYNC_NODE_STREAM_SENDER_V1
        //
        // The llama.cpp SSE reader only queues frames.
        // A dedicated worker performs authenticated HTTP delivery so
        // network latency cannot stall local token generation.
        let mut stream_sender_v1 = None;
        let mut stream_worker_v1 = None;

        if stream_requested_v1 {
            let (sender_v1, receiver_v1) = mpsc::channel::<(String, u64, Value)>();

            let task_id_for_stream_v1 = task.task_id.clone();

            let provider_for_stream_v1 = provider_email_for_task_v1.clone();

            let hardware_for_stream_v1 = hardware.clone();

            let mut stream_auth_v1 = auth.clone();

            let stream_http_v1 = Client::builder()
                // NODE_STREAM_FRAME_TIMEOUT_V2
                // Stream delivery runs on its own worker and must tolerate
                // private Realtime subscription/auth setup on the first frame.
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|_| "stream_http_client_build_failed".to_string())?;

            let worker_v1 = thread::spawn(move || {
                let stream_auth_client_v1 = match SupabaseAuthClient::from_env() {
                    Ok(client) => client,

                    Err(error) => {
                        println!("STREAM_WORKER_AUTH_CLIENT_FAILED={error}");
                        return;
                    }
                };

                for (event_v1, sequence_v1, payload_v1) in receiver_v1 {
                    match send_stream_frame_with_retry(
                        &stream_http_v1,
                        &stream_auth_client_v1,
                        &mut stream_auth_v1,
                        &task_id_for_stream_v1,
                        &provider_for_stream_v1,
                        &hardware_for_stream_v1,
                        &event_v1,
                        sequence_v1,
                        payload_v1,
                    ) {
                        Ok(status) => {
                            println!(
                                "STREAM_FRAME_HTTP_STATUS={} EVENT={} SEQUENCE={}",
                                status, event_v1, sequence_v1
                            );
                        }

                        Err(error) => {
                            println!(
                                "STREAM_WORKER_FAILED={} EVENT={} SEQUENCE={}",
                                error, event_v1, sequence_v1
                            );
                            println!("STREAMING_NON_FATAL=true");
                            break;
                        }
                    }
                }

                println!("STREAM_WORKER_STOPPED=true");
            });

            stream_sender_v1 = Some(sender_v1);

            stream_worker_v1 = Some(worker_v1);

            stream_sequence_v1 = 1;

            let queued_v1 = stream_sender_v1
                .as_ref()
                .map(|sender| {
                    sender
                        .send((
                            "generation.started".into(),
                            stream_sequence_v1,
                            json!({
                                "modelIdUsed":
                                    active_selected_model,
                                "requiredModel":
                                    task.required_model
                            }),
                        ))
                        .is_ok()
                })
                .unwrap_or(false);

            if queued_v1 {
                stream_enabled_v1 = true;

                println!("STREAM_GENERATION_STARTED_QUEUED=true");
            } else {
                println!("STREAM_GENERATION_STARTED_QUEUED=false");
            }
        }

        let use_streaming_execution_v1 = stream_enabled_v1;

        let mut stream_callback_v1 = |delta: &str| {
            if !stream_enabled_v1 {
                return;
            }

            stream_buffer_v1.push_str(delta);

            let should_flush_v1 = stream_buffer_v1.chars().count() >= 96
                || stream_last_flush_v1.elapsed() >= Duration::from_millis(250)
                || delta.contains('\n');

            if !should_flush_v1 {
                return;
            }

            let text_v1 = std::mem::take(&mut stream_buffer_v1);

            stream_sequence_v1 += 1;

            let queued_v1 = stream_sender_v1
                .as_ref()
                .map(|sender| {
                    sender
                        .send((
                            "chunk".into(),
                            stream_sequence_v1,
                            json!({
                                "text": text_v1
                            }),
                        ))
                        .is_ok()
                })
                .unwrap_or(false);

            if queued_v1 {
                stream_last_flush_v1 = Instant::now();
            } else {
                println!("STREAM_CHUNK_QUEUE_FAILED=true");

                stream_enabled_v1 = false;
                stream_buffer_v1.clear();
            }
        };

        let submit_payload = build_task_submit_payload(
            &task,
            llama.as_ref(),
            &provider_email_for_task_v1,
            &public_wallet.wallet_address,
            &hardware,
            private_key.as_str(),
            active_selected_model.as_deref(),
            active_capability.as_deref(),
            active_runtime.as_deref(),
            &runtime_acceleration,
            use_streaming_execution_v1,
            if use_streaming_execution_v1 {
                Some(&mut stream_callback_v1 as &mut dyn FnMut(&str))
            } else {
                None
            },
        )?;

        drop(stream_callback_v1);

        if (stream_enabled_v1 && !stream_buffer_v1.is_empty()) {
            let text_v1 = std::mem::take(&mut stream_buffer_v1);

            stream_sequence_v1 += 1;

            let queued_v1 = stream_sender_v1
                .as_ref()
                .map(|sender| {
                    sender
                        .send((
                            "chunk".into(),
                            stream_sequence_v1,
                            json!({
                                "text": text_v1
                            }),
                        ))
                        .is_ok()
                })
                .unwrap_or(false);

            if !queued_v1 {
                println!("STREAM_FINAL_CHUNK_QUEUE_FAILED=true");

                stream_enabled_v1 = false;
            }
        }

        if stream_enabled_v1 {
            let inference_succeeded_v1 = submit_payload
                .pointer("/payload/status")
                .and_then(Value::as_str)
                == Some("success");

            stream_sequence_v1 += 1;

            let terminal_event_v1 = if inference_succeeded_v1 {
                "generation.completed"
            } else {
                "generation.error"
            };

            let terminal_payload_v1 = if inference_succeeded_v1 {
                json!({
                    "outputComplete": true
                })
            } else {
                json!({
                    "code":
                        "neural_inference_failed"
                })
            };

            let queued_v1 = stream_sender_v1
                .as_ref()
                .map(|sender| {
                    sender
                        .send((
                            terminal_event_v1.into(),
                            stream_sequence_v1,
                            terminal_payload_v1,
                        ))
                        .is_ok()
                })
                .unwrap_or(false);

            if !queued_v1 {
                println!("STREAM_TERMINAL_QUEUE_FAILED=true");
            }
        }

        // Closing the sender drains the queue and stops the worker.
        // Join before submit-result so generation.completed cannot
        // arrive after the backend has already emitted verified ready.
        drop(stream_sender_v1);

        if let Some(worker_v1) = stream_worker_v1 {
            if worker_v1.join().is_err() {
                println!("STREAM_WORKER_JOIN_FAILED=true");
                println!("STREAMING_NON_FATAL=true");
            }
        }

        let outcome = submit_with_retry(&http, &auth_client, &mut auth, &submit_payload)?;

        println!("RESULT_SUBMIT_HTTP_STATUS={}", outcome.status);

        if outcome.status == 202
            && outcome
                .body
                .get("correctionRequested")
                .and_then(Value::as_bool)
                == Some(true)
        {
            println!("CORRECTION_REQUESTED=true");

            let correction = first_task(poll_once(
                &http,
                &auth_client,
                &mut auth,
                &hardware,
                &poll_capabilities,
            )?)
            .ok_or_else(|| "correction_redelivery_missing".to_string())?;

            if correction.task_id_text() != task_id {
                return Err("correction_wrong_task".into());
            }

            let payload = build_task_submit_payload(
                &correction,
                llama.as_ref(),
                &auth.provider_email,
                &public_wallet.wallet_address,
                &hardware,
                private_key.as_str(),
                active_selected_model.as_deref(),
                active_capability.as_deref(),
                active_runtime.as_deref(),
                &runtime_acceleration,
                false,
                None,
            )?;

            let correction_outcome = submit_with_retry(&http, &auth_client, &mut auth, &payload)?;

            println!(
                "CORRECTION_RESULT_HTTP_STATUS={}",
                correction_outcome.status
            );
        } else {
            println!("CORRECTION_REQUESTED=false");
        }

        let clear = ProductionHeartbeatV1::from_node_state(
            &state,
            env!("CARGO_PKG_VERSION"),
            "laptop",
            &[],
        );

        match send_heartbeat(&http, &auth_client, &mut auth, &clear) {
            Ok(status) => {
                println!("CLEAR_HEARTBEAT_HTTP_STATUS={status}");
                println!("CURRENT_TASK_CLEARED=true");
            }
            Err(_) => println!("CURRENT_TASK_CLEARED=false"),
        }

        println!("TASK_LIFECYCLE_COMPLETE=true");
        println!("PRIVATE_KEY_PRINTED=false");
        println!("PRIVATE_KEY_PERSISTED=false");

        if env::var("EDGESWARM_SINGLE_TASK")
            .map(|value| value.trim() == "1")
            .unwrap_or(false)
        {
            println!("SINGLE_TASK_MODE=true");
            println!("SINGLE_TASK_COMPLETE=true");
            println!("GET_JOBS_AFTER_COMPLETION=false");
            return Ok(());
        }

        // A STOP requested while a task was running takes effect
        // only after result submission and the clear heartbeat.
        if stop.load(Ordering::Acquire) {
            println!("NODE_SERVICE_STOP_REQUESTED=true");
            println!("NODE_SERVICE_STOPPED_AFTER_TASK=true");
            return Ok(());
        }

        last_idle_heartbeat = Instant::now();
        thread::sleep(Duration::from_millis(250));
    }
}
