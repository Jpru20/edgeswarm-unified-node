mod adapters;
pub mod core;
pub mod runtime;

use crate::core::{
    auth_login_client::SupabaseLoginClient,
    auth_login_contract::{jwt_aal, verified_totp_factor},
    auth_session::AuthSession,
    NodeState,
};
use serde::Serialize;
use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(AppAuthState::default()))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_node_state,
            auth_begin,
            auth_verify
        ])
        .run(tauri::generate_context!())
        .expect("error while running EdgeSwarm Unified Node");
}
