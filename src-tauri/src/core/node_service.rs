use crate::core::{
    auth_client::SupabaseAuthClient,
    backend_client::DEFAULT_BACKEND_URL,
    deterministic_executor,
    model_discovery::discover_models,
    model_provisioning::{
        fetch_model_recommendation_v1, provision_recommendation_v1, set_model_download_stage_v1,
    },
    production_heartbeat::ProductionHeartbeatV1,
    production_inference::ProductionLlamaClient,
    production_task_http::{
        poll_once, poll_once_with_limit, read_auth, send_heartbeat,
        send_stream_frame_with_retry, submit_with_retry,
    },
    real_capacity_certification::certify_model_path_v1,
    result_signing,
    task_client::{build_submit_result, GetJobsResponse, TaskEnvelope},
    wallet_account::DeviceWallet,
    wallet_client::WorkerWalletClient,
    wallet_identity::{select_wallet_row, WalletRowDecision},
    wallet_public_identity::WalletPublicIdentity,
    wallet_vault, NodeState,
};
use crate::runtime::llama_process::{
    resolve_llama_server_path_v1, resolve_model_root_v1, resolve_cpu_llama_server_path_v1, runtime_acceleration_for_config_v1, LlamaProcessConfig, ManagedLlamaProcess,
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

fn tasks_from_poll_v1(
    mut response: GetJobsResponse,
) -> Vec<TaskEnvelope> {
    if !response.tasks.is_empty() {
        return response.tasks;
    }

    response.task.take().into_iter().collect()
}

fn certified_concurrency_for_model_v1(
    state: &NodeState,
    selected_model: &str,
) -> u16 {
    state
        .models
        .iter()
        .find(|model| {
            model.selected_model == selected_model
                && model.status == "ready"
                && model.capacity_status
                    == crate::core::capacity::CapacityStatus::Certified
        })
        .and_then(|model| model.certified_concurrency)
        .unwrap_or(1)
        .max(1)
        .min(5)
}

fn apply_active_model_heartbeat_v1(
    heartbeat: &mut ProductionHeartbeatV1,
    state: &NodeState,
    selected_model: &str,
) -> Result<u16, String> {
    let model = state
        .models
        .iter()
        .find(|model| {
            model.selected_model == selected_model
                && model.status == "ready"
                && model.capacity_status
                    == crate::core::capacity::CapacityStatus::Certified
        })
        .ok_or_else(|| {
            format!(
                "active_certified_model_missing:{selected_model}"
            )
        })?;

    let concurrency = model
        .certified_concurrency
        .unwrap_or(1)
        .max(1)
        .min(5);

    heartbeat.model_id =
        Some(model.selected_model.clone());

    heartbeat.model_size_gb =
        crate::core::production_heartbeat::
            selected_model_size_gb_v1(
                selected_model
            );

    heartbeat.model_status =
        "ready".into();

    heartbeat.model_capability =
        Some(model.capability.clone());

    heartbeat.runtime =
        Some(model.runtime.clone());

    heartbeat.runtime_acceleration =
        model.acceleration.clone();

    heartbeat.concurrency_limit =
        concurrency;

    Ok(concurrency)
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

fn execution_config_for_certified_model_v1(
    model_path: String,
    certified_acceleration: &str,
) -> Result<LlamaProcessConfig, String> {
    let mut config = LlamaProcessConfig::for_model(model_path)?;

    #[cfg(target_os = "windows")]
    {
        match certified_acceleration {
            "cpu" => {
                config.executable = resolve_cpu_llama_server_path_v1()?;
                config.gpu_layers = 0;
            }
            "cuda" | "vulkan" => {
                let actual = runtime_acceleration_for_config_v1(&config);

                if actual != certified_acceleration {
                    return Err(format!(
                        "certified_runtime_acceleration_unavailable:expected={certified_acceleration}:actual={actual}"
                    ));
                }
            }
            _ => {}
        }
    }

    Ok(config)
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
            (required == active_capability || required == "Neural-Inference")
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

fn node_service_lock_contention_v1(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }

    #[cfg(target_os = "windows")]
    {
        // Windows LockFileEx may report lock/share contention as
        // ERROR_SHARING_VIOLATION (32) or ERROR_LOCK_VIOLATION (33)
        // rather than mapping it to ErrorKind::WouldBlock.
        return matches!(error.raw_os_error(), Some(32) | Some(33));
    }

    #[cfg(not(target_os = "windows"))]
    {
        false
    }
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
        Err(error) if node_service_lock_contention_v1(&error) => {
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

    let matched = disks
        .list()
        .iter()
        .filter(|disk| root.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count());

    matched
        .map(|disk| disk.available_space() / 1024 / 1024 / 1024)
        .or_else(|| {
            disks
                .list()
                .iter()
                .map(|disk| disk.available_space() / 1024 / 1024 / 1024)
                .max()
        })
        .unwrap_or(0)
}

fn model_recommendation_payload_v1(state: &NodeState) -> Value {
    let ram_gb = state.hardware.total_memory_bytes as f64 / 1024.0 / 1024.0 / 1024.0;
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
    let base_url = env::var("GCP_BASE_URL").unwrap_or_else(|_| DEFAULT_BACKEND_URL.to_string());

    println!("MODEL_RECOMMENDATION_REQUESTED=true");
    let recommendation = fetch_model_recommendation_v1(&base_url, &payload)?;

    println!("MODEL_RECOMMENDATION={}", recommendation.model_id);

    if recommendation.files.is_empty() {
        println!("MODEL_PROVISIONING_REQUIRED=false");
        return Ok(false);
    }

    println!("MODEL_PROVISIONING_REQUIRED=true");
    println!("MODEL_PROVISIONING_STARTED=true");
    let paths = provision_recommendation_v1(&recommendation)?;
    println!("MODEL_PROVISIONING_COMPLETE=true");

    if paths.is_empty() {
        return Err("provisioned_model_path_missing".to_string());
    }

    let runtime_path = resolve_llama_server_path_v1()?;

    println!("MODEL_CERTIFICATION_STARTED=true");
    println!("MODEL_CERTIFICATION_COUNT={}", paths.len());
    set_model_download_stage_v1("certifying");

    for (index, model_path) in paths.iter().enumerate() {
        println!(
            "MODEL_CERTIFICATION_ITEM={}/{}",
            index + 1,
            paths.len()
        );

        certify_model_path_v1(
            model_path.to_string_lossy().to_string(),
            runtime_path.to_string_lossy().to_string(),
        )?;

        println!(
            "MODEL_CERTIFICATION_ITEM_COMPLETE={}/{}",
            index + 1,
            paths.len()
        );
    }

    println!("MODEL_CERTIFICATION_COMPLETE=true");
    set_model_download_stage_v1("ready");

    Ok(true)
}

struct TaskWorkerOutcomeV1 {
    correction_requested: bool,
}

fn run_claimed_task_worker_v1(
    task: TaskEnvelope,
    llama: Option<ProductionLlamaClient>,
    mut auth: crate::core::production_task_http::LocalAuth,
    wallet_address: String,
    hardware: String,
    private_key: Zeroizing<String>,
    active_selected_model: Option<String>,
    active_capability: Option<String>,
    active_runtime: Option<String>,
    runtime_acceleration: String,
    poll_capabilities: Vec<String>,
    stop: Arc<AtomicBool>,
) -> Result<TaskWorkerOutcomeV1, String> {
    let auth_client = SupabaseAuthClient::from_env()?;

    let http = Client::builder()
        .timeout(Duration::from_secs(65))
        .build()
        .map_err(|_| "backend_http_client_failed".to_string())?;

    let task_id = task.task_id_text();

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
    &wallet_address,
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

let correction_requested_v1 =
    outcome.status == 202
        && outcome
            .body
            .get("correctionRequested")
            .and_then(Value::as_bool)
            == Some(true);

println!(
    "CORRECTION_REQUESTED={}",
    correction_requested_v1
);

if correction_requested_v1 {
    println!(
        "CORRECTION_REDELIVERY_DEFERRED_TO_DISPATCHER=true"
    );
}

    Ok(TaskWorkerOutcomeV1 {
        correction_requested: correction_requested_v1,
    })
}


fn run_capacity_test_at_idle_v1(
    state: &mut NodeState,
    active_selected_model: &Option<String>,
    runtime_acceleration: &mut String,
    base_heartbeat: &mut ProductionHeartbeatV1,
    managed_llama: &mut Option<ManagedLlamaProcess>,
    llama: &mut Option<ProductionLlamaClient>,
    operational_limit: &mut u16,
    single_task_mode_v1: bool,
) -> Result<(), String> {
    println!("CAPACITY_TEST_IDLE_BARRIER_REACHED=true");

    if env::var("EDGESWARM_LLAMA_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_some()
    {
        crate::core::certification_progress::
            certification_error_v1(
                "capacity_test_external_runtime_unsupported"
            );

        println!(
            "CAPACITY_TEST_REJECTED=external_runtime"
        );

        return Ok(());
    }

    let Some(selected_model_v1) =
        active_selected_model.clone()
    else {
        crate::core::certification_progress::
            certification_error_v1(
                "capacity_test_neural_model_missing"
            );

        println!(
            "CAPACITY_TEST_REJECTED=neural_model_missing"
        );

        return Ok(());
    };

    let model_path_v1 =
        match resolve_active_model_path_v1(
            &selected_model_v1
        ) {
            Ok(path) => path,
            Err(error) => {
                crate::core::certification_progress::
                    certification_error_v1(
                        error.clone()
                    );
                return Ok(());
            }
        };

    let runtime_path_v1 =
        match resolve_llama_server_path_v1() {
            Ok(path) => path,
            Err(error) => {
                crate::core::certification_progress::
                    certification_error_v1(
                        error.clone()
                    );
                return Ok(());
            }
        };

    println!("CAPACITY_TEST_PRODUCTION_RUNTIME_PAUSING=true");

    *llama = None;
    *managed_llama = None;

    thread::sleep(Duration::from_millis(250));

    println!("CAPACITY_TEST_CERTIFICATION_STARTED=true");

    let cert_result_v1 =
        std::panic::catch_unwind(
            std::panic::AssertUnwindSafe(|| {
                certify_model_path_v1(
                    model_path_v1
                        .to_string(),
                    runtime_path_v1
                        .to_string_lossy()
                        .to_string(),
                )
            }),
        );

    println!("CAPACITY_TEST_PRODUCTION_RUNTIME_RESTORING=true");

    *state = NodeState::detect();

    let restart_model_path_v1 =
        resolve_active_model_path_v1(
            &selected_model_v1
        )
        .map_err(|error| {
            format!(
                "capacity_test_runtime_restore_model_failed:{error}"
            )
        })?;

    let config_v1 =
        execution_config_for_certified_model_v1(
            restart_model_path_v1,
            runtime_acceleration.as_str(),
        )
        .map_err(|error| {
            format!(
                "capacity_test_runtime_restore_config_failed:{error}"
            )
        })?;

    let runtime_v1 =
        ManagedLlamaProcess::start(&config_v1)
            .map_err(|error| {
                format!(
                    "capacity_test_runtime_restore_start_failed:{error}"
                )
            })?;

    *runtime_acceleration =
        runtime_v1.acceleration().to_string();

    let client_v1 =
        ProductionLlamaClient::new(
            runtime_v1.base_url().to_string()
        )
        .map_err(|error| {
            format!(
                "capacity_test_runtime_restore_client_failed:{error}"
            )
        })?;

    client_v1.health_check()
        .map_err(|error| {
            format!(
                "capacity_test_runtime_restore_health_failed:{error}"
            )
        })?;

    *managed_llama = Some(runtime_v1);
    *llama = Some(client_v1);

    apply_active_model_heartbeat_v1(
        base_heartbeat,
        state,
        &selected_model_v1,
    )?;

    base_heartbeat.runtime_acceleration =
        runtime_acceleration.clone();

    *operational_limit =
        certified_concurrency_for_model_v1(
            state,
            &selected_model_v1,
        );

    if single_task_mode_v1 {
        *operational_limit = 1;
    }

    base_heartbeat.concurrency_limit =
        *operational_limit;

    println!("CAPACITY_TEST_PRODUCTION_RUNTIME_RESTORED=true");
    println!(
        "CAPACITY_TEST_NEW_OPERATIONAL_LIMIT={}",
        operational_limit
    );

    match cert_result_v1 {
        Ok(Ok(())) => {
            println!("CAPACITY_TEST_RESULT=pass");
        }

        Ok(Err(error)) => {
            crate::core::certification_progress::
                certification_error_v1(
                    error.clone()
                );

            println!(
                "CAPACITY_TEST_RESULT=failed_nonfatal|ERROR={error}"
            );
        }

        Err(_) => {
            crate::core::certification_progress::
                certification_error_v1(
                    "capacity_test_panicked"
                );

            println!(
                "CAPACITY_TEST_RESULT=panicked_nonfatal"
            );
        }
    }

    Ok(())
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

    if base_heartbeat.model_id.is_none() && provision_fresh_model_v1(&state)? {
        state = NodeState::detect();
        base_heartbeat = ProductionHeartbeatV1::from_node_state(
            &state,
            env!("CARGO_PKG_VERSION"),
            "laptop",
            &[],
        );
        println!("MODEL_STATE_REFRESHED=true");
    }

    // AUTOMATIC_CAPACITY_CERTIFICATION_V1
    // Only models lacking a valid certificate or requiring
    // revalidation are benchmarked. Valid certificates are reused.
    let auto_cert_models_v1 = state
        .models
        .iter()
        .filter(|model_v1| {
            matches!(
                model_v1.capacity_status,
                crate::core::capacity::CapacityStatus::Uncertified
                    | crate::core::capacity::CapacityStatus::RevalidationRequired
            )
        })
        .map(|model_v1| model_v1.selected_model.clone())
        .collect::<Vec<_>>();

    println!(
        "AUTOMATIC_CAPACITY_CERTIFICATION_REQUIRED={}",
        !auto_cert_models_v1.is_empty()
    );

    if !auto_cert_models_v1.is_empty() {
        let runtime_path_v1 =
            resolve_llama_server_path_v1()?
                .to_string_lossy()
                .to_string();

        set_model_download_stage_v1("certifying");

        let mut auto_cert_error_v1: Option<String> = None;

        for selected_model_v1 in auto_cert_models_v1 {
            println!(
                "AUTOMATIC_CAPACITY_CERTIFICATION_MODEL={selected_model_v1}"
            );

            let result_v1 =
                resolve_active_model_path_v1(&selected_model_v1)
                    .and_then(|model_path_v1| {
                        certify_model_path_v1(
                            model_path_v1,
                            runtime_path_v1.clone(),
                        )
                    });

            if let Err(error_v1) = result_v1 {
                println!(
                    "AUTOMATIC_CAPACITY_CERTIFICATION_ERROR={error_v1}"
                );

                crate::core::certification_progress::
                    certification_error_v1(error_v1.clone());

                auto_cert_error_v1 = Some(error_v1);
                break;
            }
        }

        if auto_cert_error_v1.is_some() {
            set_model_download_stage_v1("error");
        } else {
            set_model_download_stage_v1("ready");
        }

        state = NodeState::detect();

        base_heartbeat =
            ProductionHeartbeatV1::from_node_state(
                &state,
                env!("CARGO_PKG_VERSION"),
                "laptop",
                &[],
            );

        println!(
            "AUTOMATIC_CAPACITY_CERTIFICATION_COMPLETE={}",
            auto_cert_error_v1.is_none()
        );
    }

    let mut active_selected_model = base_heartbeat.model_id.clone();

    let mut active_capability = base_heartbeat.model_capability.clone();

    let mut active_runtime = base_heartbeat.runtime.clone();

    let mut runtime_acceleration = base_heartbeat.runtime_acceleration.clone();

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

    let mut llama = if neural_ready {
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

                let config =
                    execution_config_for_certified_model_v1(
                        model_path,
                        &runtime_acceleration,
                    )?;

                let runtime =
                    ManagedLlamaProcess::start(&config)?;

                runtime_acceleration =
                    runtime.acceleration().to_string();

                base_heartbeat.runtime_acceleration =
                    runtime_acceleration.clone();

                let base_url =
                    runtime.base_url().to_string();

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

    let single_task_mode_v1 = env::var("EDGESWARM_SINGLE_TASK")
        .map(|value| value.trim() == "1")
        .unwrap_or(false);

    let mut operational_limit_v1 = 1_u16;

    if let Some(selected_model_v1) = active_selected_model.as_deref() {
        operational_limit_v1 = apply_active_model_heartbeat_v1(
            &mut base_heartbeat,
            &state,
            selected_model_v1,
        )?;

        base_heartbeat.runtime_acceleration =
            runtime_acceleration.clone();
    }

    if single_task_mode_v1 {
        operational_limit_v1 = 1;
        base_heartbeat.concurrency_limit = 1;
    }

    println!(
        "ACTIVE_EXECUTION_CERTIFIED_CONCURRENCY={}",
        operational_limit_v1
    );

    let http = Client::builder()
        .timeout(Duration::from_secs(65))
        .build()
        .map_err(|_| "backend_http_client_failed".to_string())?;

    let readiness_status = send_heartbeat(
        &http,
        &auth_client,
        &mut auth,
        &base_heartbeat,
    )?;

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
    let mut last_block_reason: Option<String> = None;

    // Claimed tasks discovered during correction redelivery are never
    // discarded. They remain dispatcher-owned until an execution slot
    // is available.
    let mut queued_claimed_tasks_v1: Vec<TaskEnvelope> = Vec::new();

    loop {
        if stop.load(Ordering::Acquire)
            && queued_claimed_tasks_v1.is_empty()
        {
            println!("NODE_SERVICE_STOP_REQUESTED=true");
            println!("NODE_SERVICE_STOPPED=true");
            return Ok(());
        }

        if crate::core::capacity_test_control::
            capacity_test_request_pending_v1()
        {
            if !queued_claimed_tasks_v1.is_empty() {
                println!(
                    "CAPACITY_TEST_REQUEST_DEFERRED=claimed_queue"
                );
            } else {
                match crate::core::capacity_test_control::
                    take_capacity_test_request_v1()
                {
                    Ok(true) => {
                        println!(
                            "CAPACITY_TEST_REQUEST_CONSUMED=true"
                        );

                        run_capacity_test_at_idle_v1(
                            &mut state,
                            &active_selected_model,
                            &mut runtime_acceleration,
                            &mut base_heartbeat,
                            &mut _managed_llama,
                            &mut llama,
                            &mut operational_limit_v1,
                            single_task_mode_v1,
                        )?;

                        let status_v1 = send_heartbeat(
                            &http,
                            &auth_client,
                            &mut auth,
                            &base_heartbeat,
                        )?;

                        println!(
                            "CAPACITY_TEST_POST_HEARTBEAT_HTTP_STATUS={status_v1}"
                        );

                        last_idle_heartbeat =
                            Instant::now();

                        continue;
                    }

                    Ok(false) => {}

                    Err(error_v1) => {
                        crate::core::certification_progress::
                            certification_error_v1(
                                error_v1.clone()
                            );

                        println!(
                            "CAPACITY_TEST_REQUEST_ERROR={error_v1}"
                        );
                    }
                }
            }
        }

        let tasks_v1 = if !queued_claimed_tasks_v1.is_empty() {
            let take_v1 = usize::min(
                usize::from(operational_limit_v1),
                queued_claimed_tasks_v1.len(),
            );

            queued_claimed_tasks_v1
                .drain(0..take_v1)
                .collect::<Vec<_>>()
        } else {
            let poll_v1 = match poll_once_with_limit(
                &http,
                &auth_client,
                &mut auth,
                &hardware,
                &poll_capabilities,
                operational_limit_v1,
            ) {
                Ok(poll_v1) => poll_v1,

                Err(error_v1)
                    if transient_node_transport_error_v1(&error_v1) =>
                {
                    println!("POLL_TRANSIENT_ERROR={error_v1}");
                    println!("POLL_RETRYING=true");

                    for _ in 0..20 {
                        if stop.load(Ordering::Acquire) {
                            println!(
                                "NODE_SERVICE_STOP_REQUESTED=true"
                            );
                            println!("NODE_SERVICE_STOPPED=true");
                            return Ok(());
                        }

                        thread::sleep(Duration::from_millis(100));
                    }

                    continue;
                }

                Err(error_v1) => return Err(error_v1),
            };

            if poll_v1.blocked {
                let reason_v1 = poll_v1
                    .block_reason
                    .as_deref()
                    .unwrap_or("unspecified")
                    .to_string();

                if last_block_reason.as_deref()
                    != Some(reason_v1.as_str())
                {
                    println!("TASK_CLAIMED=false");
                    println!("POLL_BLOCKED=true");
                    println!("POLL_BLOCK_REASON={reason_v1}");
                    println!(
                        "POLL_BLOCK_MESSAGE={}",
                        poll_v1.message.as_deref().unwrap_or("")
                    );
                    println!(
                        "NODE_WAITING_FOR_ASSIGNMENT_APPROVAL=true"
                    );

                    last_block_reason = Some(reason_v1);
                }

                if last_idle_heartbeat.elapsed()
                    >= Duration::from_secs(15)
                {
                    let status_v1 = send_heartbeat(
                        &http,
                        &auth_client,
                        &mut auth,
                        &base_heartbeat,
                    )?;

                    println!(
                        "IDLE_HEARTBEAT_HTTP_STATUS={status_v1}"
                    );

                    last_idle_heartbeat = Instant::now();
                }

                thread::sleep(Duration::from_millis(500));
                continue;
            }

            if last_block_reason.take().is_some() {
                println!("POLL_BLOCKED=false");
                println!(
                    "NODE_WAITING_FOR_ASSIGNMENT_APPROVAL=false"
                );
                println!(
                    "POLL_ASSIGNMENT_ELIGIBILITY_RESTORED=true"
                );
            }

            tasks_from_poll_v1(poll_v1)
        };

        if tasks_v1.is_empty() {
            if last_idle_heartbeat.elapsed()
                >= Duration::from_secs(15)
            {
                match send_heartbeat(
                    &http,
                    &auth_client,
                    &mut auth,
                    &base_heartbeat,
                ) {
                    Ok(status_v1) => {
                        println!(
                            "IDLE_HEARTBEAT_HTTP_STATUS={status_v1}"
                        );
                    }

                    Err(error_v1)
                        if transient_node_transport_error_v1(
                            &error_v1
                        ) =>
                    {
                        println!(
                            "IDLE_HEARTBEAT_TRANSIENT_ERROR={error_v1}"
                        );
                    }

                    Err(error_v1) => return Err(error_v1),
                }

                last_idle_heartbeat = Instant::now();
            }

            thread::sleep(Duration::from_millis(500));
            continue;
        }

        // Determine one neural execution model for this concurrent
        // batch. Any already-claimed task targeting another model is
        // retained locally for the next drained batch.
        let current_active_model_v1 =
            active_selected_model.as_deref().and_then(|selected_v1| {
                state
                    .models
                    .iter()
                    .find(|model_v1| {
                        model_v1.selected_model == selected_v1
                    })
                    .map(|model_v1| {
                        crate::core::execution_model::ActiveModelV1 {
                            selected_model:
                                model_v1.selected_model.clone(),
                            capability:
                                model_v1.capability.clone(),
                            runtime:
                                model_v1.runtime.clone(),
                            tier:
                                model_v1.tier,
                        }
                    })
            });

        let mut batch_model_v1: Option<String> = None;
        let mut executable_tasks_v1 = Vec::new();

        for task_v1 in tasks_v1 {
            let target_v1 =
                crate::core::execution_model::certified_model_for_task(
                    &state,
                    &task_v1,
                    current_active_model_v1.as_ref(),
                )?;

            if let Some(target_v1) = target_v1 {
                if let Some(batch_model_id_v1) =
                    batch_model_v1.as_deref()
                {
                    if batch_model_id_v1
                        != target_v1.selected_model
                    {
                        println!(
                            "CLAIMED_TASK_DEFERRED_FOR_MODEL_AFFINITY={}",
                            task_v1.task_id_text()
                        );

                        queued_claimed_tasks_v1.push(task_v1);
                        continue;
                    }
                } else {
                    batch_model_v1 =
                        Some(target_v1.selected_model.clone());
                }
            }

            executable_tasks_v1.push(task_v1);
        }

        if let Some(batch_model_id_v1) =
            batch_model_v1.as_deref()
        {
            let target_state_v1 = state
                .models
                .iter()
                .find(|model_v1| {
                    model_v1.selected_model == batch_model_id_v1
                })
                .ok_or_else(|| {
                    format!(
                        "certified_model_state_missing:{}",
                        batch_model_id_v1
                    )
                })?;

            let target_v1 =
                crate::core::execution_model::ActiveModelV1 {
                    selected_model:
                        target_state_v1.selected_model.clone(),
                    capability:
                        target_state_v1.capability.clone(),
                    runtime:
                        target_state_v1.runtime.clone(),
                    tier:
                        target_state_v1.tier,
                };

            let switch_required_v1 =
                crate::core::execution_model::runtime_switch_required(
                    active_selected_model.as_deref(),
                    &target_v1,
                );

            println!(
                "MODEL_RUNTIME_SWITCH_REQUIRED={switch_required_v1}"
            );

            if switch_required_v1 {
                if env::var("EDGESWARM_LLAMA_BASE_URL")
                    .ok()
                    .filter(|value_v1| !value_v1.trim().is_empty())
                    .is_some()
                {
                    return Err(
                        "external_runtime_model_switch_unsupported".into()
                    );
                }

                llama = None;
                _managed_llama = None;

                let model_path_v1 =
                    resolve_active_model_path_v1(batch_model_id_v1)?;

                let desired_acceleration_v1 =
                    target_state_v1.acceleration.clone();

                let config_v1 =
                    execution_config_for_certified_model_v1(
                        model_path_v1,
                        &desired_acceleration_v1,
                    )?;

                let runtime_v1 =
                    ManagedLlamaProcess::start(&config_v1)?;

                runtime_acceleration =
                    runtime_v1.acceleration().to_string();

                let client_v1 = ProductionLlamaClient::new(
                    runtime_v1.base_url().to_string(),
                )?;

                client_v1.health_check()?;

                _managed_llama = Some(runtime_v1);
                llama = Some(client_v1);

                active_selected_model =
                    Some(target_v1.selected_model.clone());

                active_capability =
                    Some(target_v1.capability.clone());

                active_runtime =
                    Some(target_v1.runtime.clone());

                operational_limit_v1 =
                    certified_concurrency_for_model_v1(
                        &state,
                        batch_model_id_v1,
                    );

                if single_task_mode_v1 {
                    operational_limit_v1 = 1;
                }

                apply_active_model_heartbeat_v1(
                    &mut base_heartbeat,
                    &state,
                    batch_model_id_v1,
                )?;

                base_heartbeat.concurrency_limit =
                    operational_limit_v1;

                base_heartbeat.runtime_acceleration =
                    runtime_acceleration.clone();

                println!(
                    "ACTIVE_EXECUTION_MODEL={}",
                    batch_model_id_v1
                );

                println!(
                    "ACTIVE_EXECUTION_CERTIFIED_CONCURRENCY={}",
                    operational_limit_v1
                );

                println!("MODEL_RUNTIME_SWITCH_COMPLETE=true");
            }
        }

        if executable_tasks_v1.len()
            > usize::from(operational_limit_v1)
        {
            let overflow_v1 = executable_tasks_v1
                .split_off(usize::from(operational_limit_v1));

            let mut next_queue_v1 = overflow_v1;
            next_queue_v1.extend(queued_claimed_tasks_v1);

            queued_claimed_tasks_v1 = next_queue_v1;
        }

        if executable_tasks_v1.is_empty() {
            continue;
        }

        let active_ids_v1 = executable_tasks_v1
            .iter()
            .map(TaskEnvelope::task_id_text)
            .collect::<Vec<_>>();

        let mut active_heartbeat_v1 =
            ProductionHeartbeatV1::from_node_state(
                &state,
                env!("CARGO_PKG_VERSION"),
                "laptop",
                &active_ids_v1,
            );

        if let Some(selected_model_v1) =
            active_selected_model.as_deref()
        {
            apply_active_model_heartbeat_v1(
                &mut active_heartbeat_v1,
                &state,
                selected_model_v1,
            )?;

            active_heartbeat_v1.runtime_acceleration =
                runtime_acceleration.clone();
        }

        active_heartbeat_v1.concurrency_limit =
            operational_limit_v1;

        send_heartbeat(
            &http,
            &auth_client,
            &mut auth,
            &active_heartbeat_v1,
        )?;

        println!(
            "CONCURRENT_TASK_BATCH_SIZE={}",
            executable_tasks_v1.len()
        );

        println!(
            "CURRENT_TASK_IDS={}",
            active_ids_v1.join(",")
        );

        // TASK_WORKER_CONCURRENCY_V1
        // Workers report completion to the single dispatcher. A finished
        // slot can be refilled immediately while sibling workers continue.
        let (
            completion_tx_v1,
            completion_rx_v1
        ) = mpsc::channel::<(
            String,
            Result<TaskWorkerOutcomeV1, String>
        )>();

        let mut active_worker_ids_v1 =
            Vec::<String>::new();

        for task_v1 in executable_tasks_v1 {
            let task_id_v1 =
                task_v1.task_id_text();

            if active_worker_ids_v1
                .iter()
                .any(|active_id_v1| {
                    active_id_v1 == &task_id_v1
                })
            {
                println!(
                    "DUPLICATE_ACTIVE_TASK_SUPPRESSED={}",
                    task_id_v1
                );
                continue;
            }

            println!("TASK_CLAIMED=true");
            println!("TASK_ID={task_id_v1}");

            let tx_v1 =
                completion_tx_v1.clone();

            let llama_v1 =
                llama.clone();

            let auth_v1 =
                read_auth()?;

            let wallet_v1 =
                public_wallet.wallet_address.clone();

            let hardware_v1 =
                hardware.clone();

            let private_key_v1 =
                Zeroizing::new(
                    private_key.as_str().to_string()
                );

            let selected_model_v1 =
                active_selected_model.clone();

            let capability_v1 =
                active_capability.clone();

            let runtime_v1 =
                active_runtime.clone();

            let acceleration_v1 =
                runtime_acceleration.clone();

            let capabilities_v1 =
                poll_capabilities.clone();

            let stop_v1 =
                stop.clone();

            let task_id_for_thread_v1 =
                task_id_v1.clone();

            thread::spawn(move || {
                let result_v1 =
                    std::panic::catch_unwind(
                        std::panic::AssertUnwindSafe(|| {
                            run_claimed_task_worker_v1(
                                task_v1,
                                llama_v1,
                                auth_v1,
                                wallet_v1,
                                hardware_v1,
                                private_key_v1,
                                selected_model_v1,
                                capability_v1,
                                runtime_v1,
                                acceleration_v1,
                                capabilities_v1,
                                stop_v1,
                            )
                        })
                    );

                let outcome_v1 =
                    match result_v1 {
                        Ok(result_v1) =>
                            result_v1,

                        Err(_) =>
                            Err(format!(
                                "task_worker_panicked:{}",
                                task_id_for_thread_v1
                            ))
                    };

                let _ =
                    tx_v1.send((
                        task_id_for_thread_v1,
                        outcome_v1
                    ));
            });

            active_worker_ids_v1
                .push(task_id_v1);
        }

        while !active_worker_ids_v1.is_empty() {
            let (
                completed_task_id_v1,
                completed_outcome_v1
            ) = completion_rx_v1
                .recv()
                .map_err(|_| {
                    "task_worker_completion_channel_closed"
                        .to_string()
                })?;

            active_worker_ids_v1.retain(
                |task_id_v1|
                    task_id_v1 !=
                    &completed_task_id_v1
            );

            let completed_outcome_v1 =
                match completed_outcome_v1 {
                    Ok(outcome_v1) =>
                        Some(outcome_v1),

                    Err(error_v1) => {
                        println!(
                            "TASK_WORKER_FAILED_NON_FATAL={} ERROR={}",
                            completed_task_id_v1,
                            error_v1
                        );
                        None
                    }
                };

            auth = read_auth()?;

            println!(
                "TASK_WORKER_COMPLETED={}",
                completed_task_id_v1
            );

            let correction_requested_v1 =
                completed_outcome_v1
                    .as_ref()
                    .map(|outcome_v1| {
                        outcome_v1.correction_requested
                    })
                    .unwrap_or(false);

            // Correction redelivery remains exclusively dispatcher-owned.
            if correction_requested_v1 {
                let mut correction_v1 = None;

                for attempt_v1 in 1..=3u8 {
                    if stop.load(Ordering::Acquire) {
                        println!(
                            "CORRECTION_WAIT_STOP_REQUESTED=true"
                        );
                        break;
                    }

                    println!(
                        "CORRECTION_REDELIVERY_ATTEMPT={attempt_v1}"
                    );

                    match poll_once(
                        &http,
                        &auth_client,
                        &mut auth,
                        &hardware,
                        &poll_capabilities,
                    ) {
                        Ok(poll_v1) => {
                            for candidate_v1 in
                                tasks_from_poll_v1(poll_v1)
                            {
                                if candidate_v1.task_id_text()
                                    == completed_task_id_v1
                                {
                                    correction_v1 =
                                        Some(candidate_v1);
                                } else {
                                    let candidate_id_v1 =
                                        candidate_v1.task_id_text();

                                    let duplicate_v1 =
                                        active_worker_ids_v1
                                            .iter()
                                            .any(|active_id_v1| {
                                                active_id_v1 ==
                                                    &candidate_id_v1
                                            }) ||
                                        queued_claimed_tasks_v1
                                            .iter()
                                            .any(|queued_v1| {
                                                queued_v1
                                                    .task_id_text() ==
                                                    candidate_id_v1
                                            });

                                    if duplicate_v1 {
                                        println!(
                                            "DUPLICATE_ACTIVE_OR_QUEUED_TASK_SUPPRESSED={}",
                                            candidate_id_v1
                                        );
                                    } else {
                                        println!(
                                            "CORRECTION_UNRELATED_TASK_QUEUED={}",
                                            candidate_id_v1
                                        );

                                        queued_claimed_tasks_v1
                                            .push(candidate_v1);
                                    }
                                }
                            }
                        }

                        Err(error_v1) => {
                            println!(
                                "CORRECTION_REDELIVERY_POLL_ERROR={error_v1}"
                            );
                        }
                    }

                    if correction_v1.is_some() {
                        break;
                    }

                    if attempt_v1 < 3 {
                        thread::sleep(
                            Duration::from_millis(500)
                        );
                    }
                }

                if let Some(correction_task_v1) =
                    correction_v1
                {
                    let correction_id_v1 =
                        correction_task_v1.task_id_text();

                    let tx_v1 =
                        completion_tx_v1.clone();

                    let llama_v1 =
                        llama.clone();

                    let auth_v1 =
                        read_auth()?;

                    let wallet_v1 =
                        public_wallet.wallet_address.clone();

                    let hardware_v1 =
                        hardware.clone();

                    let private_key_v1 =
                        Zeroizing::new(
                            private_key.as_str().to_string()
                        );

                    let selected_model_v1 =
                        active_selected_model.clone();

                    let capability_v1 =
                        active_capability.clone();

                    let runtime_v1 =
                        active_runtime.clone();

                    let acceleration_v1 =
                        runtime_acceleration.clone();

                    let capabilities_v1 =
                        poll_capabilities.clone();

                    let stop_v1 =
                        stop.clone();

                    let correction_id_for_thread_v1 =
                        correction_id_v1.clone();

                    thread::spawn(move || {
                        let result_v1 =
                            std::panic::catch_unwind(
                                std::panic::AssertUnwindSafe(|| {
                                    run_claimed_task_worker_v1(
                                        correction_task_v1,
                                        llama_v1,
                                        auth_v1,
                                        wallet_v1,
                                        hardware_v1,
                                        private_key_v1,
                                        selected_model_v1,
                                        capability_v1,
                                        runtime_v1,
                                        acceleration_v1,
                                        capabilities_v1,
                                        stop_v1,
                                    )
                                })
                            );

                        let outcome_v1 =
                            match result_v1 {
                                Ok(result_v1) =>
                                    result_v1,

                                Err(_) =>
                                    Err(format!(
                                        "task_worker_panicked:{}",
                                        correction_id_for_thread_v1
                                    ))
                            };

                        let _ =
                            tx_v1.send((
                                correction_id_for_thread_v1,
                                outcome_v1
                            ));
                    });

                    active_worker_ids_v1
                        .push(correction_id_v1);

                    println!(
                        "CORRECTION_REDELIVERY_SUCCESS=true"
                    );
                } else {
                    println!(
                        "CORRECTION_REDELIVERY_TIMEOUT=true"
                    );
                    println!(
                        "CORRECTION_REDELIVERY_NON_FATAL=true"
                    );
                }
            }

            // Heartbeat immediately reflects the slot that just finished.
            let mut active_snapshot_v1 =
                ProductionHeartbeatV1::from_node_state(
                    &state,
                    env!("CARGO_PKG_VERSION"),
                    "laptop",
                    &active_worker_ids_v1,
                );

            if let Some(selected_model_v1) =
                active_selected_model.as_deref()
            {
                apply_active_model_heartbeat_v1(
                    &mut active_snapshot_v1,
                    &state,
                    selected_model_v1,
                )?;

                active_snapshot_v1
                    .runtime_acceleration =
                    runtime_acceleration.clone();
            }

            active_snapshot_v1.concurrency_limit =
                operational_limit_v1;

            send_heartbeat(
                &http,
                &auth_client,
                &mut auth,
                &active_snapshot_v1,
            )?;

            if stop.load(Ordering::Acquire)
                || single_task_mode_v1
                || !queued_claimed_tasks_v1.is_empty()
                || active_worker_ids_v1.is_empty()
            {
                continue;
            }

            let free_slots_v1 =
                usize::from(operational_limit_v1)
                    .saturating_sub(
                        active_worker_ids_v1.len()
                    );

            if free_slots_v1 == 0 {
                continue;
            }

            let refill_poll_v1 =
                match poll_once_with_limit(
                    &http,
                    &auth_client,
                    &mut auth,
                    &hardware,
                    &poll_capabilities,
                    free_slots_v1 as u16,
                ) {
                    Ok(poll_v1) =>
                        poll_v1,

                    Err(error_v1)
                        if transient_node_transport_error_v1(
                            &error_v1
                        ) =>
                    {
                        println!(
                            "REFILL_POLL_TRANSIENT_ERROR={error_v1}"
                        );
                        continue;
                    }

                    Err(error_v1) =>
                        return Err(error_v1),
                };

            if refill_poll_v1.blocked {
                println!(
                    "REFILL_POLL_BLOCKED=true"
                );
                continue;
            }

            for refill_task_v1 in
                tasks_from_poll_v1(refill_poll_v1)
            {
                let refill_id_v1 =
                    refill_task_v1.task_id_text();

                let duplicate_v1 =
                    active_worker_ids_v1
                        .iter()
                        .any(|active_id_v1| {
                            active_id_v1 ==
                                &refill_id_v1
                        }) ||
                    queued_claimed_tasks_v1
                        .iter()
                        .any(|queued_v1| {
                            queued_v1.task_id_text() ==
                                refill_id_v1
                        });

                if duplicate_v1 {
                    println!(
                        "DUPLICATE_ACTIVE_OR_QUEUED_TASK_SUPPRESSED={}",
                        refill_id_v1
                    );
                    continue;
                }

                if active_worker_ids_v1.len()
                    >= usize::from(
                        operational_limit_v1
                    )
                {
                    queued_claimed_tasks_v1
                        .push(refill_task_v1);
                    continue;
                }

                let current_model_v1 =
                    active_selected_model
                        .as_deref()
                        .and_then(|selected_v1| {
                            state.models
                                .iter()
                                .find(|model_v1| {
                                    model_v1.selected_model
                                        == selected_v1
                                })
                                .map(|model_v1| {
                                    crate::core::execution_model::ActiveModelV1 {
                                        selected_model:
                                            model_v1.selected_model.clone(),
                                        capability:
                                            model_v1.capability.clone(),
                                        runtime:
                                            model_v1.runtime.clone(),
                                        tier:
                                            model_v1.tier,
                                    }
                                })
                        });

                let refill_target_v1 =
                    crate::core::execution_model::certified_model_for_task(
                        &state,
                        &refill_task_v1,
                        current_model_v1.as_ref(),
                    )?;

                if let Some(target_v1) =
                    refill_target_v1.as_ref()
                {
                    if active_selected_model
                        .as_deref()
                        != Some(
                            target_v1
                                .selected_model
                                .as_str()
                        )
                    {
                        println!(
                            "REFILL_DEFERRED_FOR_MODEL_AFFINITY={}",
                            refill_task_v1.task_id_text()
                        );

                        queued_claimed_tasks_v1
                            .push(refill_task_v1);

                        continue;
                    }
                }

                let tx_v1 =
                    completion_tx_v1.clone();

                let llama_v1 =
                    llama.clone();

                let auth_v1 =
                    read_auth()?;

                let wallet_v1 =
                    public_wallet.wallet_address.clone();

                let hardware_v1 =
                    hardware.clone();

                let private_key_v1 =
                    Zeroizing::new(
                        private_key.as_str().to_string()
                    );

                let selected_model_v1 =
                    active_selected_model.clone();

                let capability_v1 =
                    active_capability.clone();

                let runtime_v1 =
                    active_runtime.clone();

                let acceleration_v1 =
                    runtime_acceleration.clone();

                let capabilities_v1 =
                    poll_capabilities.clone();

                let stop_v1 =
                    stop.clone();

                let refill_id_for_thread_v1 =
                    refill_id_v1.clone();

                thread::spawn(move || {
                    let result_v1 =
                        std::panic::catch_unwind(
                            std::panic::AssertUnwindSafe(|| {
                                run_claimed_task_worker_v1(
                                    refill_task_v1,
                                    llama_v1,
                                    auth_v1,
                                    wallet_v1,
                                    hardware_v1,
                                    private_key_v1,
                                    selected_model_v1,
                                    capability_v1,
                                    runtime_v1,
                                    acceleration_v1,
                                    capabilities_v1,
                                    stop_v1,
                                )
                            })
                        );

                    let outcome_v1 =
                        match result_v1 {
                            Ok(result_v1) =>
                                result_v1,

                            Err(_) =>
                                Err(format!(
                                    "task_worker_panicked:{}",
                                    refill_id_for_thread_v1
                                ))
                        };

                    let _ =
                        tx_v1.send((
                            refill_id_for_thread_v1,
                            outcome_v1
                        ));
                });

                active_worker_ids_v1
                    .push(refill_id_v1);

                println!(
                    "TASK_SLOT_REFILLED_IMMEDIATELY=true"
                );
            }

            let mut refill_snapshot_v1 =
                ProductionHeartbeatV1::from_node_state(
                    &state,
                    env!("CARGO_PKG_VERSION"),
                    "laptop",
                    &active_worker_ids_v1,
                );

            if let Some(selected_model_v1) =
                active_selected_model.as_deref()
            {
                apply_active_model_heartbeat_v1(
                    &mut refill_snapshot_v1,
                    &state,
                    selected_model_v1,
                )?;

                refill_snapshot_v1
                    .runtime_acceleration =
                    runtime_acceleration.clone();
            }

            refill_snapshot_v1.concurrency_limit =
                operational_limit_v1;

            send_heartbeat(
                &http,
                &auth_client,
                &mut auth,
                &refill_snapshot_v1,
            )?;
        }

        auth = read_auth()?;

        let mut clear_v1 =
            ProductionHeartbeatV1::from_node_state(
                &state,
                env!("CARGO_PKG_VERSION"),
                "laptop",
                &[],
            );

        if let Some(selected_model_v1) =
            active_selected_model.as_deref()
        {
            apply_active_model_heartbeat_v1(
                &mut clear_v1,
                &state,
                selected_model_v1,
            )?;

            clear_v1.runtime_acceleration =
                runtime_acceleration.clone();
        }

        clear_v1.concurrency_limit =
            operational_limit_v1;

        match send_heartbeat(
            &http,
            &auth_client,
            &mut auth,
            &clear_v1,
        ) {
            Ok(status_v1) => {
                println!(
                    "CLEAR_HEARTBEAT_HTTP_STATUS={status_v1}"
                );
                println!("CURRENT_TASK_CLEARED=true");
            }

            Err(_) => {
                println!("CURRENT_TASK_CLEARED=false");
            }
        }

        println!("TASK_LIFECYCLE_COMPLETE=true");
        println!("PRIVATE_KEY_PRINTED=false");
        println!("PRIVATE_KEY_PERSISTED=false");

        if single_task_mode_v1 {
            println!("SINGLE_TASK_MODE=true");
            println!("SINGLE_TASK_COMPLETE=true");
            println!("GET_JOBS_AFTER_COMPLETION=false");
            return Ok(());
        }

        last_idle_heartbeat = Instant::now();
        thread::sleep(Duration::from_millis(100));
    }
}
