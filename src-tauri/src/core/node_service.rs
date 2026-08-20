use crate::core::{
    auth_client::SupabaseAuthClient,
    deterministic_executor,
    model_discovery::discover_models,
    production_heartbeat::ProductionHeartbeatV1,
    production_inference::ProductionLlamaClient,
    production_task_http::{poll_once, read_auth, send_heartbeat, submit_with_retry},
    result_signing,
    task_client::{build_submit_result, GetJobsResponse, TaskEnvelope},
    wallet_account::DeviceWallet,
    wallet_client::WorkerWalletClient,
    wallet_identity::{select_wallet_row, WalletRowDecision},
    wallet_public_identity::WalletPublicIdentity,
    wallet_vault, NodeState,
};
use crate::runtime::llama_process::{
    resolve_model_root_v1, LlamaProcessConfig, ManagedLlamaProcess,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

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
            .unwrap_or(0);

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

    match llama.execute(&task.prompt, task.max_output_tokens) {
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

pub fn run_node_service(
    stop: Arc<AtomicBool>,
    mut wallet_password: Zeroizing<String>,
) -> Result<(), String> {
    let auth_client = SupabaseAuthClient::from_env()?;
    auth_client.ensure_valid_session(true)?;

    let mut auth = read_auth()?;
    let state = NodeState::detect();
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
        let heartbeat = ProductionHeartbeatV1::from_node_state(&state, "0.1.0", "laptop", &[]);

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

    let base_heartbeat = ProductionHeartbeatV1::from_node_state(&state, "0.1.0", "laptop", &[]);

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

        let poll = poll_once(
            &http,
            &auth_client,
            &mut auth,
            &hardware,
            &poll_capabilities,
        )?;

        if poll.blocked {
            println!("TASK_CLAIMED=false");
            println!("POLL_BLOCKED=true");
            return Ok(());
        }

        let Some(task) = first_task(poll) else {
            if last_idle_heartbeat.elapsed() >= Duration::from_secs(15) {
                let status = send_heartbeat(&http, &auth_client, &mut auth, &base_heartbeat)?;
                println!("IDLE_HEARTBEAT_HTTP_STATUS={status}");
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

        let mut active = ProductionHeartbeatV1::from_node_state(&state, "0.1.0", "laptop", &[]);

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

        let submit_payload = build_task_submit_payload(
            &task,
            llama.as_ref(),
            &auth.provider_email,
            &public_wallet.wallet_address,
            &hardware,
            private_key.as_str(),
            active_selected_model.as_deref(),
            active_capability.as_deref(),
            active_runtime.as_deref(),
            &runtime_acceleration,
        )?;

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
            )?;

            let correction_outcome = submit_with_retry(&http, &auth_client, &mut auth, &payload)?;

            println!(
                "CORRECTION_RESULT_HTTP_STATUS={}",
                correction_outcome.status
            );
        } else {
            println!("CORRECTION_REQUESTED=false");
        }

        let clear = ProductionHeartbeatV1::from_node_state(&state, "0.1.0", "laptop", &[]);

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
