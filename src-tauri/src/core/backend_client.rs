use crate::core::auth_session::AuthSession;
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
};
use serde::Serialize;
use std::{env, time::Duration};

pub const DEFAULT_BACKEND_URL: &str =
    "https://api.edgeswarm.io";

#[derive(Debug)]
pub struct BackendClient {
    base_url: String,
    http: Client,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendClientDryRun {
    pub base_url: String,
    pub heartbeat_url: String,
    pub auth_session_present: bool,
    pub auth_session_valid: bool,
    pub provider_email_present: bool,
    pub bearer_header_ready: bool,
    pub network_request_sent: bool,
}

impl BackendClient {
    pub fn new() -> Result<Self, String> {
        let base_url = env::var("GCP_BASE_URL")
            .unwrap_or_else(|_| {
                DEFAULT_BACKEND_URL.to_string()
            })
            .trim_end_matches('/')
            .to_string();

        let http = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| {
                "backend_http_client_build_failed"
                    .to_string()
            })?;

        Ok(Self {
            base_url,
            http,
        })
    }

    pub fn heartbeat_url(&self) -> String {
        format!(
            "{}/admin/node-heartbeat",
            self.base_url
        )
    }

    pub fn authenticated_headers(
        session: &AuthSession,
    ) -> Result<HeaderMap, String> {
        if !session.is_valid_now() {
            return Err(
                "auth_session_refresh_required"
                    .to_string()
            );
        }

        let token = session
            .access_token()
            .ok_or_else(|| {
                "auth_access_token_missing"
                    .to_string()
            })?;

        let mut value =
            HeaderValue::from_str(
                &format!("Bearer {token}")
            )
            .map_err(|_| {
                "auth_bearer_header_invalid"
                    .to_string()
            })?;

        value.set_sensitive(true);

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value);

        Ok(headers)
    }

    pub fn dry_run(
        &self,
    ) -> Result<BackendClientDryRun, String> {
        let session =
            AuthSession::load_default()?;

        let bearer_ready =
            Self::authenticated_headers(&session)
                .is_ok();

        Ok(BackendClientDryRun {
            base_url:
                self.base_url.clone(),
            heartbeat_url:
                self.heartbeat_url(),
            auth_session_present: true,
            auth_session_valid:
                session.is_valid_now(),
            provider_email_present:
                session.provider_email().is_some(),
            bearer_header_ready:
                bearer_ready,
            network_request_sent:
                false,
        })
    }

    pub fn http(&self) -> &Client {
        &self.http
    }
}
