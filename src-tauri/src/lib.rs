mod adapters;
pub mod core;
pub mod runtime;

use crate::core::{
    auth_client::SupabaseAuthClient,
    auth_login_client::SupabaseLoginClient,
    auth_login_contract::{jwt_aal, verified_totp_factor},
    auth_session::AuthSession,
    node_service::{clear_node_service_logs, node_service_logs, run_node_service},
    NodeState,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
        Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

#[derive(Default)]
struct AppAuthState {
    pending_email: Option<String>,
    pending_access_token: Option<Zeroizing<String>>,
    pending_factor_id: Option<String>,
    pending_challenge_id: Option<String>,

    authenticated_email: Option<String>,

    // Kept only in Rust process memory so the future node service can
    // unlock the encrypted device wallet without prompting in a terminal.
    wallet_password: Option<Zeroizing<String>>,
}

struct NodeRuntimeState {
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    last_error: Arc<Mutex<Option<String>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Default for NodeRuntimeState {
    fn default() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            last_error: Arc::new(Mutex::new(None)),
            worker: Mutex::new(None),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeServiceStatus {
    running: bool,
    stopping: bool,
    last_error: Option<String>,
    logs: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthBeginResult {
    email: String,
    mfa_required: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthVerifyResult {
    email: String,
}

// PROVIDER_LEDGER_SYNC_COMMAND_V1
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderLedgerApiResponse {
    total_earned_usd: f64,
    synced_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderLedgerSummary {
    total_earned_usd: f64,
    synced_at: Option<String>,
}

fn provider_api_base_v1() -> String {
    env::var("GCP_BASE_URL")
        .unwrap_or_else(|_| "https://api.edgeswarm.io".into())
        .trim_end_matches('/')
        .to_string()
}

fn fetch_provider_ledger_v1(
    access_token: &str,
) -> Result<ProviderLedgerApiResponse, String> {
    let response = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| "provider_ledger_http_client_failed".to_string())?
        .get(format!(
            "{}/v1/provider/ledger/me",
            provider_api_base_v1()
        ))
        .bearer_auth(access_token)
        .send()
        .map_err(|_| "provider_ledger_network_failed".to_string())?;

    let status = response.status().as_u16();

    if !(200..300).contains(&status) {
        return Err(format!("provider_ledger_http_{status}"));
    }

    response
        .json::<ProviderLedgerApiResponse>()
        .map_err(|_| "provider_ledger_response_invalid".to_string())
}

#[tauri::command]
fn provider_ledger_sync() -> Result<ProviderLedgerSummary, String> {
    let auth_client = SupabaseAuthClient::from_env()?;
    let ensured = auth_client.ensure_valid_session(true)?;

    let access_token = ensured
        .session
        .access_token()
        .ok_or_else(|| "provider_ledger_access_token_missing".to_string())?;

    let response = match fetch_provider_ledger_v1(access_token) {
        Err(error) if error == "provider_ledger_http_401" => {
            let refreshed = auth_client.force_refresh_session()?;
            let token = refreshed
                .session
                .access_token()
                .ok_or_else(|| {
                    "provider_ledger_refreshed_token_missing".to_string()
                })?;

            fetch_provider_ledger_v1(token)?
        }
        Err(error) => return Err(error),
        Ok(response) => response,
    };

    if !response.total_earned_usd.is_finite()
        || response.total_earned_usd < 0.0
    {
        return Err("provider_ledger_usd_invalid".into());
    }

    Ok(ProviderLedgerSummary {
        total_earned_usd: response.total_earned_usd,
        synced_at: response.synced_at,
    })
}

#[tauri::command]
fn set_window_layout(
    window: tauri::Window,
    screen: String,
) -> Result<(), String> {
    let (width, height) = if screen == "dashboard" {
        if cfg!(target_os = "macos") {
            (560.0, 680.0)
        } else {
            (560.0, 560.0)
        }
    } else {
        (600.0, 360.0)
    };

    window
        .set_size(tauri::Size::Logical(
            tauri::LogicalSize::new(width, height),
        ))
        .map_err(|error| format!("window_resize_failed:{error}"))
}
#[tauri::command]
fn get_node_state() -> NodeState {
    NodeState::detect()
}

#[tauri::command]
fn auth_begin(
    email: String,
    password: String,
    auth_state: tauri::State<'_, Mutex<AppAuthState>>,
) -> Result<AuthBeginResult, String> {
    let email = email.trim().to_lowercase();

    if email.is_empty() || password.is_empty() {
        return Err("email_or_password_missing".into());
    }

    let login = SupabaseLoginClient::from_env()?;
    let aal1 = login.password_login(&email, &password)?;
    let user = login.get_user(&aal1.access_token)?;

    let authenticated_email = user
        .email
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_lowercase();

    if authenticated_email != email {
        return Err("authenticated_email_mismatch".into());
    }

    let factor_id = verified_totp_factor(&user)
        .ok_or_else(|| "verified_totp_factor_missing".to_string())?
        .id
        .clone();

    let challenge =
        login.challenge(&aal1.access_token, &factor_id)?;

    let mut state = auth_state
        .lock()
        .map_err(|_| "auth_state_lock_failed".to_string())?;

    state.pending_email = Some(authenticated_email.clone());
    state.pending_access_token =
        Some(Zeroizing::new(aal1.access_token));
    state.pending_factor_id = Some(factor_id);
    state.pending_challenge_id = Some(challenge.id);

    // Preserve the login password only inside Rust memory.
    // The existing wallet encryption flow uses this password to unlock
    // the device wallet after successful MFA.
    state.wallet_password =
        Some(Zeroizing::new(password));

    Ok(AuthBeginResult {
        email: authenticated_email,
        mfa_required: true,
    })
}

#[tauri::command]
fn auth_verify(
    code: String,
    auth_state: tauri::State<'_, Mutex<AppAuthState>>,
) -> Result<AuthVerifyResult, String> {
    let code = code.trim();

    if code.len() != 6 ||
        !code.chars().all(|c| c.is_ascii_digit())
    {
        return Err("invalid_mfa_code_format".into());
    }

    let (
        email,
        access_token,
        factor_id,
        challenge_id,
    ) = {
        let state = auth_state
            .lock()
            .map_err(|_| "auth_state_lock_failed".to_string())?;

        (
            state.pending_email.clone(),
            state
                .pending_access_token
                .as_ref()
                .map(|v| v.to_string()),
            state.pending_factor_id.clone(),
            state.pending_challenge_id.clone(),
        )
    };

    let email =
        email.ok_or_else(|| "pending_auth_email_missing".to_string())?;

    let access_token = access_token
        .ok_or_else(|| "pending_auth_access_token_missing".to_string())?;

    let factor_id = factor_id
        .ok_or_else(|| "pending_auth_factor_missing".to_string())?;

    let challenge_id = challenge_id
        .ok_or_else(|| "pending_auth_challenge_missing".to_string())?;

    let login = SupabaseLoginClient::from_env()?;

    let verified = login.verify(
        &access_token,
        &factor_id,
        &challenge_id,
        code,
    )?;

    if jwt_aal(&verified.access_token).as_deref() != Some("aal2") {
        return Err("mfa_session_not_aal2".into());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "clock_failed".to_string())?
        .as_secs();

    let expires_at = verified
        .expires_at
        .or_else(|| {
            verified
                .expires_in
                .map(|seconds| now.saturating_add(seconds))
        })
        .ok_or_else(|| "mfa_session_expiry_missing".to_string())?;

    let session = AuthSession::from_authenticated_session(
        &email,
        &verified.access_token,
        &verified.refresh_token,
        expires_at,
    )?;

    session.save_secure()?;

    {
        let mut state = auth_state
            .lock()
            .map_err(|_| "auth_state_lock_failed".to_string())?;

        state.pending_email = None;
        state.pending_access_token = None;
        state.pending_factor_id = None;
        state.pending_challenge_id = None;

        state.authenticated_email = Some(email.clone());

        // wallet_password intentionally remains until logout/app exit
        // or until we explicitly zeroize it after the node service owns it.
    }

    Ok(AuthVerifyResult { email })
}

fn current_node_service_status(
    runtime: &NodeRuntimeState,
) -> Result<NodeServiceStatus, String> {
    let running = runtime.running.load(Ordering::Acquire);
    let stopping =
        running && runtime.stop.load(Ordering::Acquire);

    let last_error = runtime
        .last_error
        .lock()
        .map_err(|_| "node_service_error_lock_failed".to_string())?
        .clone();

    let logs = node_service_logs();

    Ok(NodeServiceStatus {
        running,
        stopping,
        last_error,
        logs,
    })
}

#[tauri::command]
fn node_service_status(
    runtime: tauri::State<'_, NodeRuntimeState>,
) -> Result<NodeServiceStatus, String> {
    current_node_service_status(&runtime)
}

#[tauri::command]
fn start_node(
    auth_state: tauri::State<'_, Mutex<AppAuthState>>,
    runtime: tauri::State<'_, NodeRuntimeState>,
) -> Result<NodeServiceStatus, String> {
    if runtime.running.load(Ordering::Acquire) {
        return current_node_service_status(&runtime);
    }

    // Reap a previously completed worker before starting another.
    {
        let mut worker = runtime
            .worker
            .lock()
            .map_err(|_| "node_worker_lock_failed".to_string())?;

        if worker
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(false)
        {
            if let Some(handle) = worker.take() {
                let _ = handle.join();
            }
        }
    }

    let wallet_password = {
        let state = auth_state
            .lock()
            .map_err(|_| "auth_state_lock_failed".to_string())?;

        if state.authenticated_email.is_none() {
            return Err("node_start_requires_authenticated_session".into());
        }

        let password = state
            .wallet_password
            .as_ref()
            .ok_or_else(|| {
                "node_start_wallet_password_missing".to_string()
            })?;

        Zeroizing::new(password.as_str().to_owned())
    };

    clear_node_service_logs();
    runtime.stop.store(false, Ordering::Release);

    {
        let mut error = runtime
            .last_error
            .lock()
            .map_err(|_| "node_service_error_lock_failed".to_string())?;

        *error = None;
    }

    runtime.running.store(true, Ordering::Release);

    let stop = Arc::clone(&runtime.stop);
    let running = Arc::clone(&runtime.running);
    let last_error = Arc::clone(&runtime.last_error);

    let handle = thread::spawn(move || {
        let _power_guard =
            match crate::core::power_guard::PowerGuard::acquire() {
                Ok(guard) => guard,
                Err(error) => {
                    if let Ok(mut slot) = last_error.lock() {
                        *slot = Some(error);
                    }

                    running.store(false, Ordering::Release);
                    return;
                }
            };

        let result = run_node_service(
            Arc::clone(&stop),
            wallet_password,
        );

        if let Err(error) = result {
            if let Ok(mut slot) = last_error.lock() {
                *slot = Some(error);
            }
        }

        running.store(false, Ordering::Release);
    });

    {
        let mut worker = runtime
            .worker
            .lock()
            .map_err(|_| "node_worker_lock_failed".to_string())?;

        *worker = Some(handle);
    }

    current_node_service_status(&runtime)
}

#[tauri::command]
fn stop_node(
    runtime: tauri::State<'_, NodeRuntimeState>,
) -> Result<NodeServiceStatus, String> {
    if runtime.running.load(Ordering::Acquire) {
        runtime.stop.store(true, Ordering::Release);
    }

    current_node_service_status(&runtime)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(AppAuthState::default()))
        .manage(NodeRuntimeState::default())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_node_state,
            set_window_layout,
            auth_begin,
            auth_verify,
            provider_ledger_sync,
            node_service_status,
            start_node,
            stop_node
        ])
        .run(tauri::generate_context!())
        .expect("error while running EdgeSwarm Unified Node");
}
