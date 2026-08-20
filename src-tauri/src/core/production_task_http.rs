use crate::adapters;
use crate::core::{
    auth_client::SupabaseAuthClient,
    production_heartbeat::ProductionHeartbeatV1,
    task_client::{build_poll_url, GetJobsResponse},
};
use reqwest::{
    blocking::Client,
    header::{HeaderValue, AUTHORIZATION},
};
use serde_json::Value;
use std::{
    env,
    fs,
    path::PathBuf,
    thread,
    time::Duration,
};

#[derive(Clone)]
pub struct LocalAuth {
    pub provider_email: String,
    pub access_token: String,
}

pub struct SubmitOutcome {
    pub status: u16,
    pub body: Value,
}

fn auth_path() -> Result<PathBuf, String> {
    if let Ok(path) = env::var("EDGESWARM_AUTH_FILE") {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    Ok(
        adapters::app_data_dir()
            .join("auth_session.json")
    )
}

pub fn read_auth() -> Result<LocalAuth, String> {
    let raw = fs::read_to_string(auth_path()?)
        .map_err(|_| "auth_session_read_failed".to_string())?;

    let value: Value =
        serde_json::from_str(&raw)
            .map_err(|_| "auth_session_parse_failed".to_string())?;

    if value.get("mfaVerified").and_then(Value::as_bool) != Some(true) {
        return Err("auth_session_mfa_not_verified".into());
    }

    let provider_email = value
        .get("providerEmail")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();

    let access_token = value
        .get("accessToken")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if provider_email.is_empty() || access_token.is_empty() {
        return Err("auth_session_identity_missing".into());
    }

    Ok(LocalAuth {
        provider_email,
        access_token,
    })
}

fn bearer(token: &str) -> Result<HeaderValue, String> {
    let mut value =
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "bearer_header_invalid".to_string())?;

    value.set_sensitive(true);
    Ok(value)
}

fn api_base() -> String {
    env::var("GCP_BASE_URL")
        .unwrap_or_else(|_| "https://api.edgeswarm.io".into())
        .trim_end_matches('/')
        .to_string()
}

pub fn refresh_auth(
    client: &SupabaseAuthClient,
) -> Result<LocalAuth, String> {
    client.force_refresh_session()?;
    read_auth()
}

pub fn send_heartbeat(
    http: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    heartbeat: &ProductionHeartbeatV1,
) -> Result<u16, String> {
    for auth_try in 0..2 {
        let response = http
            .post(format!("{}/admin/node-heartbeat", api_base()))
            .header(AUTHORIZATION, bearer(&auth.access_token)?)
            .json(heartbeat)
            .send()
            .map_err(|_| "heartbeat_network_failed".to_string())?;

        let status = response.status().as_u16();

        if status == 401 && auth_try == 0 {
            *auth = refresh_auth(auth_client)?;
            continue;
        }

        if status == 426 {
            return Err("node_update_required_http_426".into());
        }

        if !(200..300).contains(&status) {
            return Err(format!("heartbeat_http_{status}"));
        }

        return Ok(status);
    }

    Err("heartbeat_auth_failed_after_refresh".into())
}

pub fn poll_once(
    http: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    hardware_id: &str,
    capabilities: &[String],
) -> Result<GetJobsResponse, String> {
    let url = build_poll_url(
        hardware_id,
        &auth.provider_email,
        capabilities,
        env!("CARGO_PKG_VERSION"),
        adapters::platform_name(),
    )?;

    for auth_try in 0..2 {
        let response = http
            .get(url.clone())
            .header(AUTHORIZATION, bearer(&auth.access_token)?)
            .send()
            .map_err(|_| "task_poll_network_failed".to_string())?;

        let status = response.status().as_u16();

        if status == 401 && auth_try == 0 {
            *auth = refresh_auth(auth_client)?;
            continue;
        }

        if status == 426 {
            return Err("node_update_required_http_426".into());
        }

        if status != 200 {
            return Err(format!("task_poll_http_{status}"));
        }

        return response
            .json::<GetJobsResponse>()
            .map_err(|_| "task_poll_response_invalid".to_string());
    }

    Err("task_poll_auth_failed_after_refresh".into())
}

pub fn submit_with_retry(
    http: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    payload: &Value,
) -> Result<SubmitOutcome, String> {
    let mut auth_refreshed = false;

    for attempt in 1..=3 {
        let response = match http
            .post(format!(
                "{}/enterprise/submit-result",
                api_base()
            ))
            .header(AUTHORIZATION, bearer(&auth.access_token)?)
            .json(payload)
            .send()
        {
            Ok(response) => response,

            Err(_) if attempt < 3 => {
                thread::sleep(Duration::from_secs(attempt));
                continue;
            }

            Err(_) => {
                return Err(
                    "result_submit_network_failed_after_retries".into()
                );
            }
        };

        let status = response.status().as_u16();

        if status == 401 && !auth_refreshed {
            *auth = refresh_auth(auth_client)?;
            auth_refreshed = true;
            continue;
        }

        if status == 426 {
            return Err("node_update_required_http_426".into());
        }

        let raw = response.text().unwrap_or_default();

        let body =
            serde_json::from_str::<Value>(&raw)
                .unwrap_or(Value::Null);

        if matches!(status, 200 | 201 | 202) {
            return Ok(SubmitOutcome { status, body });
        }

        let retryable =
            matches!(status, 408 | 425 | 429)
            || (500..=599).contains(&status);

        if retryable && attempt < 3 {
            thread::sleep(Duration::from_secs(attempt));
            continue;
        }

        return Ok(SubmitOutcome { status, body });
    }

    Err("result_submit_retry_exhausted".into())
}
