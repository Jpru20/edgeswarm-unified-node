use edgeswarm_unified_node_lib::core::{
    auth_client::SupabaseAuthClient,
    production_heartbeat::ProductionHeartbeatV1,
    production_inference::ProductionLlamaClient,
    production_task_http::{
        poll_once, read_auth, send_heartbeat,
        submit_with_retry,
    },
    result_signing,
    task_client::{build_submit_result, GetJobsResponse, TaskEnvelope},
    wallet_account::DeviceWallet,
    wallet_client::WorkerWalletClient,
    wallet_identity::{select_wallet_row, WalletRowDecision},
    wallet_public_identity::WalletPublicIdentity,
    wallet_vault,
    NodeState,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{env, thread, time::{Duration, Instant}};
use zeroize::Zeroizing;

fn first_task(mut r: GetJobsResponse) -> Option<TaskEnvelope> {
    if !r.tasks.is_empty() {
        Some(r.tasks.remove(0))
    } else {
        r.task
    }
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
    }).to_string();

    let hash = Sha256::digest(output.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    let signature = result_signing::sign_result(
        &task.task_id_text(),
        0,
        &hash,
        hardware,
        private_key,
    )?;

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
            "modelIdUsed": "qwen2.5:3b",
            "runtime": "llama.cpp",
            "runtimeAcceleration": "cpu"
        }
    }))
}

fn run() -> Result<(), String> {
    let execute = env::var("EXECUTE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !execute {
        println!("EXECUTE=false");
        println!("RUNNER_MODE=continuous_limit_1");
        println!("REAL_WALLET_UNLOCKED=false");
        println!("NETWORK_REQUEST_SENT=false");
        println!("HEARTBEAT_SENT=false");
        println!("TASK_POLL_SENT=false");
        println!("TASK_CLAIMED=false");
        println!("RESULT_SUBMITTED=false");
        return Ok(());
    }

    let auth_client = SupabaseAuthClient::from_env()?;
    auth_client.ensure_valid_session(true)?;

    let mut auth = read_auth()?;
    let state = NodeState::detect();
    let hardware = state.hardware_identity.hardware_id.clone();

    let public_wallet = WalletPublicIdentity::load_default()?;

    if public_wallet.hardware_id != hardware {
        return Err("wallet_hardware_mismatch".into());
    }

    let wallet_client = WorkerWalletClient::from_env()?;
    let rows = wallet_client.rows_for_email(
        &auth.access_token,
        &auth.provider_email,
    )?;

    let row_index = match select_wallet_row(&rows, &hardware)? {
        WalletRowDecision::ExactDevice { row_index } => row_index,
        WalletRowDecision::ClaimLegacy { .. } =>
            return Err("runner_refuses_legacy_wallet".into()),
        WalletRowDecision::CreateDevice =>
            return Err("runner_refuses_wallet_creation".into()),
    };

    let password = Zeroizing::new(
        rpassword::prompt_password("Wallet password: ")
            .map_err(|_| "wallet_password_read_failed".to_string())?
    );

    let private_key = Zeroizing::new(
        wallet_vault::decrypt(
            &rows[row_index].private_key,
            &password,
            &auth.provider_email,
        )?
    );

    let recovered =
        DeviceWallet::from_private_key(private_key.as_str())?;

    if !recovered.wallet_address()
        .eq_ignore_ascii_case(&public_wallet.wallet_address)
    {
        return Err("wallet_unlock_identity_mismatch".into());
    }

    println!("WALLET_UNLOCKED=true");

    let base_heartbeat =
        ProductionHeartbeatV1::from_node_state(
            &state, "0.1.0", "laptop", &[],
        );

    if base_heartbeat.concurrency_limit != 1
        || base_heartbeat.eligible_model_capabilities
            != vec!["Neural-Inference-3B".to_string()]
    {
        return Err("runner_capacity_gate_failed".into());
    }

    let llama = ProductionLlamaClient::new(
        env::var("EDGESWARM_LLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:18081".into())
    )?;

    // Critical: runtime must be ready BEFORE get-jobs can claim.
    llama.health_check()?;
    println!("LOCAL_RUNTIME_READY=true");

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

    println!(
        "READINESS_HEARTBEAT_HTTP_STATUS={readiness_status}"
    );

    let mut last_idle_heartbeat = Instant::now();

    loop {
    let poll = poll_once(
        &http,
        &auth_client,
        &mut auth,
        &hardware,
        &base_heartbeat.eligible_model_capabilities,
    )?;

    if poll.blocked {
        println!("TASK_CLAIMED=false");
        println!("POLL_BLOCKED=true");
        return Ok(());
    }

    let Some(task) = first_task(poll) else {
        if last_idle_heartbeat.elapsed() >= Duration::from_secs(15) {
            let status = send_heartbeat(
                &http,
                &auth_client,
                &mut auth,
                &base_heartbeat,
            )?;
            println!("IDLE_HEARTBEAT_HTTP_STATUS={status}");
            last_idle_heartbeat = Instant::now();
        }

        thread::sleep(Duration::from_secs(1));
        continue;
    };

    let task_id = task.task_id_text();

    println!("TASK_CLAIMED=true");
    println!("TASK_ID={task_id}");

    let mut active =
        ProductionHeartbeatV1::from_node_state(
            &state, "0.1.0", "laptop", &[],
        );

    active.current_task_ids = vec![task_id.clone()];

    match send_heartbeat(
        &http, &auth_client, &mut auth, &active,
    ) {
        Ok(status) =>
            println!("ACTIVE_HEARTBEAT_HTTP_STATUS={status}"),

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

            let outcome = submit_with_retry(
                &http,
                &auth_client,
                &mut auth,
                &failure,
            )?;

            println!(
                "FAILURE_RESULT_HTTP_STATUS={}",
                outcome.status
            );
            println!("TASK_LIFECYCLE_STOPPED_AFTER_HEARTBEAT_FAILURE=true");
            return Ok(());
        }
    }

    let required =
        task.required_model.as_deref().unwrap_or("");

    let selected =
        task.selected_model.as_deref().unwrap_or("");

    let supported =
        required == "Neural-Inference-3B"
        && (
            selected.is_empty()
            || selected == "qwen2.5:3b"
            || selected == "tier:auto"
        );

    let submit_payload = if supported {
        match llama.execute(
            &task.prompt,
            task.max_output_tokens,
        ) {
            Ok(result) => {
                println!("INFERENCE_SUCCEEDED=true");
                println!("INFERENCE_LATENCY_MS={}", result.latency_ms);

                serde_json::to_value(
                    build_submit_result(
                        &task,
                        &result.ai_output,
                        &auth.provider_email,
                        &public_wallet.wallet_address,
                        &hardware,
                        private_key.as_str(),
                        result.latency_ms,
                        "qwen2.5:3b",
                        "llama.cpp",
                        "cpu",
                    )?
                )
                .map_err(|_| "result_payload_encode_failed".to_string())?
            }

            Err(_) => {
                println!("INFERENCE_SUCCEEDED=false");

                failure_payload(
                    &task,
                    &auth.provider_email,
                    &public_wallet.wallet_address,
                    &hardware,
                    private_key.as_str(),
                    "neural_inference_failed",
                )?
            }
        }
    } else {
        println!("TASK_SUPPORTED=false");

        failure_payload(
            &task,
            &auth.provider_email,
            &public_wallet.wallet_address,
            &hardware,
            private_key.as_str(),
            "unsupported_claimed_task",
        )?
    };

    let outcome = submit_with_retry(
        &http,
        &auth_client,
        &mut auth,
        &submit_payload,
    )?;

    println!("RESULT_SUBMIT_HTTP_STATUS={}", outcome.status);

    if outcome.status == 202
        && outcome.body
            .get("correctionRequested")
            .and_then(Value::as_bool)
            == Some(true)
    {
        println!("CORRECTION_REQUESTED=true");

        let correction = first_task(
            poll_once(
                &http,
                &auth_client,
                &mut auth,
                &hardware,
                &base_heartbeat.eligible_model_capabilities,
            )?
        )
        .ok_or_else(|| "correction_redelivery_missing".to_string())?;

        if correction.task_id_text() != task_id {
            return Err("correction_wrong_task".into());
        }

        let result = llama.execute(
            &correction.prompt,
            correction.max_output_tokens,
        )?;

        let payload = serde_json::to_value(
            build_submit_result(
                &correction,
                &result.ai_output,
                &auth.provider_email,
                &public_wallet.wallet_address,
                &hardware,
                private_key.as_str(),
                result.latency_ms,
                "qwen2.5:3b",
                "llama.cpp",
                "cpu",
            )?
        )
        .map_err(|_| "correction_payload_encode_failed".to_string())?;

        let correction_outcome =
            submit_with_retry(
                &http,
                &auth_client,
                &mut auth,
                &payload,
            )?;

        println!(
            "CORRECTION_RESULT_HTTP_STATUS={}",
            correction_outcome.status
        );
    } else {
        println!("CORRECTION_REQUESTED=false");
    }

    let clear =
        ProductionHeartbeatV1::from_node_state(
            &state, "0.1.0", "laptop", &[],
        );

    match send_heartbeat(
        &http,
        &auth_client,
        &mut auth,
        &clear,
    ) {
        Ok(status) => {
            println!("CLEAR_HEARTBEAT_HTTP_STATUS={status}");
            println!("CURRENT_TASK_CLEARED=true");
        }
        Err(_) => println!("CURRENT_TASK_CLEARED=false"),
    }

    println!("TASK_LIFECYCLE_COMPLETE=true");
    println!("PRIVATE_KEY_PRINTED=false");
    println!("PRIVATE_KEY_PERSISTED=false");

    last_idle_heartbeat = Instant::now();
    thread::sleep(Duration::from_millis(250));
    }
}

fn main() {
    if let Err(error) = run() {
        println!(
            "PRODUCTION_TASK_RUNNER_ERROR={}",
            error.replace('\n', " ")
        );
    }
}
