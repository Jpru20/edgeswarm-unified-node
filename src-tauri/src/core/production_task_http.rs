use crate::adapters;
use crate::core::{
    auth_client::SupabaseAuthClient,
    production_heartbeat::ProductionHeartbeatV1,
    task_client::{build_poll_url_with_limit, GetJobsResponse},
};
use reqwest::{
    blocking::Client,
    header::{HeaderValue, AUTHORIZATION},
};
use serde_json::Value;
use std::{
    env,
    fs,
    io::Read,
    path::PathBuf,
    sync::{Mutex, OnceLock},
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

static AUTH_REFRESH_LOCK_V1: OnceLock<Mutex<()>> = OnceLock::new();

fn auth_refresh_lock_v1() -> &'static Mutex<()> {
    AUTH_REFRESH_LOCK_V1.get_or_init(|| Mutex::new(()))
}

fn auth_path() -> Result<PathBuf, String> {
    if let Ok(path) = env::var("EDGESWARM_AUTH_FILE") {
        if !path.trim().is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    Ok(adapters::app_data_dir().join("auth_session.json"))
}

pub fn read_auth() -> Result<LocalAuth, String> {
    let raw =
        fs::read_to_string(auth_path()?).map_err(|_| "auth_session_read_failed".to_string())?;

    let value: Value =
        serde_json::from_str(&raw).map_err(|_| "auth_session_parse_failed".to_string())?;

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

pub(crate) fn bearer(token: &str) -> Result<HeaderValue, String> {
    let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "bearer_header_invalid".to_string())?;

    value.set_sensitive(true);
    Ok(value)
}

pub(crate) fn api_base() -> String {
    env::var("GCP_BASE_URL")
        .unwrap_or_else(|_| "https://api.edgeswarm.io".into())
        .trim_end_matches('/')
        .to_string()
}

pub fn refresh_auth(
    client: &SupabaseAuthClient,
    stale_access_token: &str,
) -> Result<LocalAuth, String> {
    let _guard = auth_refresh_lock_v1()
        .lock()
        .map_err(|_| "auth_refresh_lock_poisoned".to_string())?;

    // Another worker may already have refreshed while this caller
    // waited for the lock. Reuse the rotated session instead of
    // rotating the refresh token a second time.
    let current = read_auth()?;

    if current.access_token != stale_access_token {
        return Ok(current);
    }

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
        // LIVE_UPDATE_LIFECYCLE_SEND_REFRESH_V1
        //
        // Always serialize a fresh lifecycle snapshot. The caller may
        // reuse its heartbeat object for many minutes.
        let mut live_heartbeat =
            heartbeat.clone();

        live_heartbeat
            .refresh_update_lifecycle_v1();

        let response = http
            .post(format!("{}/admin/node-heartbeat", api_base()))
            .header(AUTHORIZATION, bearer(&auth.access_token)?)
            .json(&live_heartbeat)
            .send()
            .map_err(|_| "heartbeat_network_failed".to_string())?;

        let status = response.status().as_u16();

        if status == 401 && auth_try == 0 {
            let stale_access_token = auth.access_token.clone();
            *auth = refresh_auth(
                auth_client,
                &stale_access_token,
            )?;
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
    poll_once_with_limit(
        http,
        auth_client,
        auth,
        hardware_id,
        capabilities,
        1,
    )
}

fn record_swct_result_ready_v170(
    response: &mut GetJobsResponse,
    ready: bool,
) {
    if !ready { return; }
    if let Some(task) = response.task.as_mut() {
        task.swct_result_ready_v170 = true;
    }
    for task in &mut response.tasks {
        task.swct_result_ready_v170 = true;
    }
}
fn decode_task_poll_body_v170(
    content_type: &str,
    body: &[u8],
) -> Result<GetJobsResponse, String> {
    let media = content_type.split(';').next().unwrap_or("").trim();

    let raw = if media.eq_ignore_ascii_case("application/vnd.swarm.swct.v1") {
        crate::core::swarm_compact_transport_v1::decode(
            crate::core::swarm_compact_transport_v1::Kind::Task,
            body,
        ).map_err(|_| "task_poll_swct_invalid".to_string())?
    } else if media.is_empty() || media.eq_ignore_ascii_case("application/json") {
        body.to_vec()
    } else {
        return Err("task_poll_media_type_invalid".into());
    };

    serde_json::from_slice(&raw)
        .map_err(|_| "task_poll_response_invalid".into())
}

pub fn poll_once_with_limit(
    http: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    hardware_id: &str,
    capabilities: &[String],
    limit: u16,
) -> Result<GetJobsResponse, String> {
    let url = build_poll_url_with_limit(
        hardware_id,
        &auth.provider_email,
        capabilities,
        env!("CARGO_PKG_VERSION"),
        adapters::platform_name(),
        limit,
    )?;

    for auth_try in 0..2 {
        let swct_v170 =
            !matches!(env::var("SWARM_SWCT_TASK_POLL_CANARY_V170").ok().as_deref(), Some("0") | Some("false"));

        let mut request = http.get(url.clone())
            .header(AUTHORIZATION, bearer(&auth.access_token)?);

        if swct_v170 {
            request = request.header(
                reqwest::header::ACCEPT,
                "application/vnd.swarm.swct.v1, application/json",
            );
        }

        let response = request.send()
            .map_err(|_| "task_poll_network_failed".to_string())?;

        let status = response.status().as_u16();

        if status == 401 && auth_try == 0 {
            let stale_access_token = auth.access_token.clone();
            *auth = refresh_auth(
                auth_client,
                &stale_access_token,
            )?;
            continue;
        }

        if status == 426 {
            return Err("node_update_required_http_426".into());
        }

        if status != 200 {
            return Err(format!("task_poll_http_{status}"));
        }

        let result_ready_v170 = swct_v170
            && response.headers()
                .get("x-swarm-swct-result-ready")
                .and_then(|value| value.to_str().ok())
                == Some("1");

        if swct_v170 {
            const MAX_BODY: usize = (2 << 20) + (64 << 10) + 48;
            let media = response.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("").to_owned();

            let mut reader = response.take((MAX_BODY + 1) as u64);
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes)
                .map_err(|_| "task_poll_body_read_failed".to_string())?;

            if bytes.len() > MAX_BODY {
                return Err("task_poll_body_too_large".into());
            }
            let mut decoded_v170 =
                decode_task_poll_body_v170(&media, &bytes)?;
            record_swct_result_ready_v170(
                &mut decoded_v170, result_ready_v170
            );
            return Ok(decoded_v170);
        }

        return response
            .json::<GetJobsResponse>()
            .map_err(|_| "task_poll_response_invalid".to_string());
    }

    Err("task_poll_auth_failed_after_refresh".into())
}

// NODE_STREAM_FRAME_HTTP_V1
// Sends transient generation frames using the same authenticated
// provider session as get-jobs and submit-result.
pub fn send_stream_frame_with_retry(
    http: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    task_id: &Value,
    provider_email: &str,
    hardware_id: &str,
    event: &str,
    sequence: u64,
    payload: Value,
) -> Result<u16, String> {
    let body = serde_json::json!({
        "taskId": task_id,
        "providerEmail": provider_email,
        "hardwareId": hardware_id,
        "event": event,
        "sequence": sequence,
        "payload": payload
    });

    let mut auth_refreshed = false;

    for attempt in 1..=2 {
        let response = match http
            .post(format!("{}/enterprise/task-stream-frame", api_base()))
            .header(AUTHORIZATION, bearer(&auth.access_token)?)
            .json(&body)
            .send()
        {
            Ok(response) => response,

            Err(error) if attempt < 2 => {
                println!(
                    "STREAM_FRAME_SEND_RETRY={} ATTEMPT={}",
                    if error.is_timeout() {
                        "timeout"
                    } else {
                        "network"
                    },
                    attempt
                );

                thread::sleep(Duration::from_millis(100));
                continue;
            }

            Err(error) => {
                return Err(if error.is_timeout() {
                    "stream_frame_timeout"
                } else {
                    "stream_frame_network_failed"
                }
                .into());
            }
        };

        let status = response.status().as_u16();

        if (status == 401 && !auth_refreshed) {
            let stale_access_token = auth.access_token.clone();
            *auth = refresh_auth(
                auth_client,
                &stale_access_token,
            )?;

            auth_refreshed = true;
            continue;
        }

        if status == 426 {
            return Err("node_update_required_http_426".into());
        }

        if matches!(status, 200 | 201 | 202) {
            return Ok(status);
        }

        let retryable = matches!(status, 408 | 425 | 429) || (500..=599).contains(&status);

        if retryable && attempt < 2 {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let raw = response.text().unwrap_or_default();

        return Err(format!(
            "stream_frame_http_{}:{}",
            status,
            raw.chars().take(240).collect::<String>()
        ));
    }

    Err("stream_frame_retry_exhausted".into())
}


// ADAPTIVE_EXECUTION_PROGRESS_HTTP_V1
// Sends execution-liveness telemetry to the backend.
// Generated response text is not included in this channel.
pub fn send_task_progress_with_retry(
    http: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    task_id: &Value,
    provider_email: &str,
    hardware_id: &str,
    execution_id: &str,
    event: &str,
    sequence: u64,
    output_chars: u64,
    elapsed_ms: u64,
) -> Result<u16, String> {
    let body = serde_json::json!({
        "taskId": task_id,
        "providerEmail": provider_email,
        "hardwareId": hardware_id,
        "executionId": execution_id,
        "event": event,
        "sequence": sequence,
        "outputChars": output_chars,
        "elapsedMs": elapsed_ms
    });

    let mut auth_refreshed = false;

    for attempt in 1..=2 {
        let response = match http
            .post(format!(
                "{}/enterprise/task-progress",
                api_base()
            ))
            .header(
                AUTHORIZATION,
                bearer(&auth.access_token)?
            )
            .json(&body)
            .send()
        {
            Ok(response) => response,

            Err(error) if attempt < 2 => {
                println!(
                    "TASK_PROGRESS_SEND_RETRY={} ATTEMPT={}",
                    if error.is_timeout() {
                        "timeout"
                    } else {
                        "network"
                    },
                    attempt
                );

                thread::sleep(
                    Duration::from_millis(100)
                );

                continue;
            }

            Err(error) => {
                return Err(
                    if error.is_timeout() {
                        "task_progress_timeout"
                    } else {
                        "task_progress_network_failed"
                    }
                    .into()
                );
            }
        };

        let status =
            response.status().as_u16();

        if status == 401 && !auth_refreshed {
            let stale_access_token =
                auth.access_token.clone();

            *auth = refresh_auth(
                auth_client,
                &stale_access_token,
            )?;

            auth_refreshed = true;
            continue;
        }

        if status == 426 {
            return Err(
                "node_update_required_http_426".into()
            );
        }

        if matches!(status, 200 | 201 | 202) {
            return Ok(status);
        }

        let retryable =
            matches!(status, 408 | 425 | 429)
                || (500..=599).contains(&status);

        if retryable && attempt < 2 {
            thread::sleep(
                Duration::from_millis(100)
            );

            continue;
        }

        let raw =
            response.text().unwrap_or_default();

        return Err(format!(
            "task_progress_http_{}:{}",
            status,
            raw.chars()
                .take(240)
                .collect::<String>()
        ));
    }

    Err(
        "task_progress_retry_exhausted".into()
    )
}
#[allow(dead_code)]
struct ResultWireV170 {
    media_type: &'static str,
    body: Vec<u8>,
}

// Prepared only; the production submit path remains JSON.
#[allow(dead_code)]
fn prepare_result_wire_v170(
    payload: &Value,
    backend_support_confirmed: bool,
) -> Result<ResultWireV170, String> {
    use crate::core::swarm_compact_transport_v1::{choose, Kind, Transfer};

    let raw = serde_json::to_vec(payload)
        .map_err(|_| "result_json_encode_failed".to_string())?;

    let legacy = |body| ResultWireV170 {
        media_type: "application/json",
        body,
    };

    if !backend_support_confirmed {
        return Ok(legacy(raw));
    }

    match choose(Kind::Result, &raw)
        .map_err(|e| e.to_string())?
    {
        Transfer::Legacy(bytes) => Ok(legacy(bytes)),
        Transfer::Compact(bytes) => Ok(ResultWireV170 {
            media_type: "application/vnd.swarm.swct.v1",
            body: bytes,
        }),
    }
}

#[cfg(test)]
mod swct_result_wire_v170_tests {
    use super::*;

    #[test]
    fn existing_json_result_unchanged() {
        let value = serde_json::json!({
            "fileHash": "abc123",
            "payload": {
                "taskId": "task-170",
                "attemptId": "attempt-1",
                "signature": "synthetic-signature",
                "aiOutput": "test"
            }
        });
        let prepared = prepare_result_wire_v170(&value, false).unwrap();
        assert_eq!(prepared.media_type, "application/json");
        assert_eq!(prepared.body, serde_json::to_vec(&value).unwrap());
    }

    #[test]
    fn compact_result_preserves_exact_signed_envelope() {
        use crate::core::swarm_compact_transport_v1::{decode, Kind};

        let value = serde_json::json!({
            "fileHash": "abc123",
            "payload": {
                "taskId": "task-170",
                "attemptId": "attempt-1",
                "signature": "synthetic-signature",
                "aiOutput": "Synthetic result text. ".repeat(500)
            }
        });

        let prepared = prepare_result_wire_v170(&value, true).unwrap();
        assert_eq!(
            prepared.media_type,
            "application/vnd.swarm.swct.v1"
        );

        let decoded = decode(Kind::Result, &prepared.body).unwrap();
        assert_eq!(decoded, serde_json::to_vec(&value).unwrap());

        let mut damaged = prepared.body.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(decode(Kind::Result, &damaged).is_err());
    }
}
pub fn submit_with_retry(
    http: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    payload: &Value,
    result_ready_v170: bool,
) -> Result<SubmitOutcome, String> {
    let mut auth_refreshed = false;

    let allowed_v170 = result_ready_v170
        && !matches!(env::var("SWARM_SWCT_RESULT_SUBMIT_CANARY_V170").ok().as_deref(), Some("0") | Some("false"));

    let wire_v170 = if allowed_v170 {
        prepare_result_wire_v170(payload, true).ok()
    } else {
        None
    };

    let use_compact_v170 = wire_v170.as_ref()
        .map(|wire| wire.media_type ==
            "application/vnd.swarm.swct.v1")
        .unwrap_or(false);

    for attempt in 1..=3 {
        let mut request_v170 = http
            .post(format!("{}/enterprise/submit-result", api_base()))
            .header(AUTHORIZATION, bearer(&auth.access_token)?);

        if use_compact_v170 {
            if let Some(wire) = wire_v170.as_ref() {
                request_v170 = request_v170
                    .header(reqwest::header::CONTENT_TYPE, wire.media_type)
                    .body(wire.body.clone());
            }
        } else {
            request_v170 = request_v170.json(payload);
        }

        let response = match request_v170.send()
        {
            Ok(response) => response,

            Err(_) if attempt < 3 => {
                thread::sleep(Duration::from_secs(attempt));
                continue;
            }

            Err(_) => {
                return Err("result_submit_network_failed_after_retries".into());
            }
        };

        let status = response.status().as_u16();

        if status == 401 && !auth_refreshed {
            let stale_access_token = auth.access_token.clone();
            *auth = refresh_auth(
                auth_client,
                &stale_access_token,
            )?;
            auth_refreshed = true;
            continue;
        }

        if status == 426 {
            return Err("node_update_required_http_426".into());
        }

        let raw = response.text().unwrap_or_default();

        let body = serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null);

        if status == 415 && use_compact_v170
            && body.get("error").and_then(Value::as_str)
                == Some("swct_disabled")
        {
            return submit_with_retry(
                http, auth_client, auth, payload, false
            );
        }

        if matches!(status, 200 | 201 | 202) {
            return Ok(SubmitOutcome { status, body });
        }

        let retryable = matches!(status, 408 | 425 | 429) || (500..=599).contains(&status);

        if retryable && attempt < 3 {
            thread::sleep(Duration::from_secs(attempt));
            continue;
        }

        return Ok(SubmitOutcome { status, body });
    }

    Err("result_submit_retry_exhausted".into())
}

#[cfg(test)]
mod swct_poll_http_v170_tests {
    use super::*;
    use crate::core::swarm_compact_transport_v1::{encode, Kind};

    const RAW: &[u8] =
        br#"{"task":{"taskId":"swct-170","prompt":"test"}}"#;

    #[test]
    fn accepts_legacy_json() {
        let decoded = decode_task_poll_body_v170("application/json", RAW).unwrap();
        assert_eq!(decoded.task.unwrap().task_id_text(), "swct-170");
    }

    #[test]
    fn accepts_compact_task() {
        let frame = encode(Kind::Task, RAW).unwrap();
        let decoded = decode_task_poll_body_v170(
            "application/vnd.swarm.swct.v1", &frame
        ).unwrap();
        assert_eq!(decoded.task.unwrap().task_id_text(), "swct-170");
    }

    #[test]
    fn rejects_wrong_kind_and_corruption() {
        let wrong = encode(Kind::Result, RAW).unwrap();
        assert!(decode_task_poll_body_v170(
            "application/vnd.swarm.swct.v1", &wrong
        ).is_err());

        let mut damaged = encode(Kind::Task, RAW).unwrap();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(decode_task_poll_body_v170(
            "application/vnd.swarm.swct.v1", &damaged
        ).is_err());
    }

    #[test]
    fn rejects_unexpected_media_type() {
        assert!(decode_task_poll_body_v170(
            "application/octet-stream", RAW
        ).is_err());
    }
}

#[cfg(test)]
mod swct_local_http_v170 {
    use super::*;
    use std::{io::{Read, Write}, net::TcpListener};

    #[test]
    fn negotiated_transport_over_local_http() {
        let raw = br#"{"task":{"taskId":"mock-170","prompt":"synthetic"}}"#;
        let cases = vec![
            ("legacy", false, "application/json", raw.to_vec(), true),
            ("fallback", true, "application/json", raw.to_vec(), true),
            ("compact", true, "application/vnd.swarm.swct.v1",
                crate::core::swarm_compact_transport_v1::encode(
                    crate::core::swarm_compact_transport_v1::Kind::Task,
                    raw,
                ).unwrap(), true),
            ("corrupt", true, "application/vnd.swarm.swct.v1",
                b"invalid-swct-frame".to_vec(), false),
        ];

        for (label, offer, media, body, expected_ok) in cases {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();

            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut received = Vec::new();
                let mut buf = [0u8; 1024];

                loop {
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0);
                    received.extend_from_slice(&buf[..n]);
                    assert!(received.len() <= 8192);
                    if received.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }

                let request = String::from_utf8_lossy(&received)
                    .to_ascii_lowercase();
                assert_eq!(
                    request.contains(
                        "accept: application/vnd.swarm.swct.v1, application/json"
                    ),
                    offer
                );

                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).unwrap();
                socket.write_all(&body).unwrap();
            });

            let client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build().unwrap();

            let mut request = client.get(format!("http://{address}/swarm/get-jobs"));
            if offer {
                request = request.header(
                    reqwest::header::ACCEPT,
                    "application/vnd.swarm.swct.v1, application/json",
                );
            }

            let response = request.send().unwrap();
            let content_type = response.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .unwrap().to_str().unwrap().to_owned();
            let body = response.bytes().unwrap();

            let result = decode_task_poll_body_v170(&content_type, &body);
            server.join().unwrap();

            assert_eq!(result.is_ok(), expected_ok, "{label}");
            if let Ok(task) = result {
                assert_eq!(
                    task.task.unwrap().task_id_text(),
                    "mock-170"
                );
            }
            println!("LOCAL_HTTP_{label}=PASS");
        }
    }
}


#[cfg(test)]
mod swct_result_http_local_v170 {
    use super::*;
    use std::{io::{Read, Write}, net::TcpListener};

    #[test]
    fn outbound_json_and_swct_over_local_http() {
        for (supported, small) in [
            (false, false), (true, false), (true, true)
        ] {
            let output = if small {
                "ok".to_owned()
            } else {
                "Synthetic result evidence. ".repeat(500)
            };

            let payload = serde_json::json!({
                "fileHash": "synthetic-hash",
                "payload": {
                    "taskId": "test-170",
                    "attemptId": "attempt-1",
                    "signature": "synthetic-signature",
                    "aiOutput": output
                }
            });

            let original = serde_json::to_vec(&payload).unwrap();
            let wire = prepare_result_wire_v170(
                &payload, supported
            ).unwrap();

            let compact = supported && !small;
            assert_eq!(
                wire.media_type == "application/vnd.swarm.swct.v1",
                compact
            );

            let expected_media = wire.media_type.to_owned();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();

            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(
                    Some(Duration::from_secs(5))
                ).unwrap();

                let mut received = Vec::new();
                let mut buffer = [0u8; 8192];

                let (header_end, body_len) = loop {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    received.extend_from_slice(&buffer[..n]);
                    assert!(received.len() < 2_000_000);

                    if let Some(pos) = received.windows(4)
                        .position(|w| w == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(
                            &received[..pos]
                        ).to_ascii_lowercase();

                        assert!(headers.starts_with(
                            "post /enterprise/submit-result "
                        ));
                        assert!(headers.contains(&format!(
                            "content-type: {}",
                            expected_media
                        )));

                        let length = headers.lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            }).unwrap();

                        if received.len() >= pos + 4 + length {
                            break (pos + 4, length);
                        }
                    }
                };

                let body = &received[
                    header_end..header_end + body_len
                ];

                let decoded = if compact {
                    crate::core::swarm_compact_transport_v1::decode(
                        crate::core::swarm_compact_transport_v1::Kind::Result,
                        body,
                    ).unwrap()
                } else {
                    body.to_vec()
                };

                assert_eq!(decoded, original);

                socket.write_all(
                    b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}"
                ).unwrap();
            });

            let client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build().unwrap();

            let response = client
                .post(format!(
                    "http://{address}/enterprise/submit-result"
                ))
                .header(
                    reqwest::header::CONTENT_TYPE,
                    wire.media_type,
                )
                .body(wire.body)
                .send().unwrap();

            assert_eq!(response.status().as_u16(), 201);
            assert_eq!(
                response.json::<Value>().unwrap()["ok"],
                true
            );

            server.join().unwrap();

            println!(
                "RESULT_HTTP supported={supported} small={small} PASS"
            );
        }
    }
}


#[cfg(test)]
mod swct_result_handshake_v170_tests {
    use super::*;

    #[test]
    fn readiness_is_attached_only_to_claimed_tasks() {
        let mut response: GetJobsResponse =
            serde_json::from_value(serde_json::json!({
                "task": {
                    "taskId": "t170",
                    "prompt": "test",
                    "swctResultReadyV170": true
                }
            })).unwrap();

        assert!(!response.task.as_ref().unwrap()
            .swct_result_ready_v170);

        record_swct_result_ready_v170(&mut response, true);

        assert!(response.task.unwrap().swct_result_ready_v170);
    }

    #[test]
    fn no_handshake_means_no_result_permission() {
        let mut response: GetJobsResponse =
            serde_json::from_value(serde_json::json!({
                "tasks": [{"taskId": "t171", "prompt": "test"}]
            })).unwrap();

        record_swct_result_ready_v170(&mut response, false);

        assert!(!response.tasks[0].swct_result_ready_v170);
    }
}


#[cfg(test)]
mod swct_actual_http_v170 {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn receive(mut stream: &TcpStream) -> (String, Vec<u8>) {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0 && bytes.len() < 500_000);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(p) = bytes.windows(4)
                .position(|w| w == b"\r\n\r\n")
            {
                let header = String::from_utf8_lossy(&bytes[..p])
                    .to_ascii_lowercase();
                let len = header.lines()
                    .find_map(|line| line.strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
                if bytes.len() >= p + 4 + len {
                    return (header, bytes[p + 4..p + 4 + len].to_vec());
                }
            }
        }
    }

    #[test]
    #[ignore = "isolated localhost test; temporarily overrides environment"]
    fn swct_actual_http_roundtrip_v170() {
        use crate::core::swarm_compact_transport_v1::{encode, decode, Kind};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());

        let payload = serde_json::json!({
            "fileHash": "synthetic-hash",
            "payload": {
                "taskId": "swct-170",
                "signature": "synthetic-signature",
                "aiOutput": "Synthetic evidence. ".repeat(600)
            }
        });

        let expected = serde_json::to_vec(&payload).unwrap();

        let server = std::thread::spawn(move || {
            for phase in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(
                    Some(Duration::from_secs(8))
                ).unwrap();

                let (headers, body) = receive(&socket);

                if phase == 0 {
                    assert!(headers.starts_with("get /swarm/get-jobs?"));
                    assert!(headers.contains(
                        "accept: application/vnd.swarm.swct.v1, application/json"
                    ));

                    let raw = br#"{"task":{"taskId":"swct-170","prompt":"synthetic"}}"#;
                    let frame = encode(Kind::Task, raw).unwrap();
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.swarm.swct.v1\r\nX-Swarm-SWCT-Result-Ready: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        frame.len()
                    );
                    socket.write_all(reply.as_bytes()).unwrap();
                    socket.write_all(&frame).unwrap();
                } else {
                    assert!(headers.starts_with(
                        "post /enterprise/submit-result "
                    ));
                    assert!(headers.contains(
                        "content-type: application/vnd.swarm.swct.v1"
                    ));
                    assert_eq!(
                        decode(Kind::Result, &body).unwrap(),
                        expected
                    );
                    socket.write_all(
                        b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}"
                    ).unwrap();
                }
            }
        });

        let settings = [
            ("GCP_BASE_URL", base),
            ("SWARM_SWCT_TASK_POLL_CANARY_V170", "1".into()),
            ("SWARM_SWCT_RESULT_SUBMIT_CANARY_V170", "1".into()),
        ];
        let previous: Vec<_> = settings.iter()
            .map(|(key, _)| std::env::var(key).ok())
            .collect();

        for (key, value) in &settings {
            std::env::set_var(key, value);
        }

        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(8))
            .build().unwrap();

        let auth_client = SupabaseAuthClient::from_env().unwrap();
        let mut auth = LocalAuth {
            provider_email: "test@example.com".into(),
            access_token: "synthetic-token".into(),
        };

        let response = poll_once_with_limit(
            &client, &auth_client, &mut auth,
            "synthetic-hardware", &[], 1
        ).unwrap();

        let task = response.task.unwrap();
        assert_eq!(task.task_id_text(), "swct-170");
        assert!(task.swct_result_ready_v170);
        println!("ACTUAL_COMPACT_POLL=PASS");
        println!("RESULT_HANDSHAKE_CAPTURED=PASS");

        let outcome = submit_with_retry(
            &client, &auth_client, &mut auth,
            &payload, task.swct_result_ready_v170
        ).unwrap();

        assert_eq!(outcome.status, 201);
        assert_eq!(outcome.body["ok"], true);
        server.join().unwrap();

        for ((key, _), old) in settings.iter().zip(previous) {
            match old {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }

        println!("ACTUAL_COMPACT_RESULT_SUBMISSION=PASS");
        println!("SIGNED_ENVELOPE_BYTES_PRESERVED=PASS");
    }
}
