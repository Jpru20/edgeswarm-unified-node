use crate::core::{
    auth_client::SupabaseAuthClient,
    auth_session::AuthSession,
    backend_client::BackendClient,
    production_heartbeat::ProductionHeartbeatV1,
};
use serde::Serialize;

#[derive(Debug)]
pub struct ProductionHeartbeatClient {
    backend: BackendClient,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionHeartbeatSendResultV1 {
    pub status_code: u16,
    pub auth_refresh_attempted: bool,
    pub network_request_sent: bool,
    pub heartbeat_sent: bool,
    pub update_required: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionHeartbeatDryRunV1 {
    pub heartbeat_url: String,
    pub auth_session_valid: bool,
    pub provider_email_present: bool,
    pub bearer_header_ready: bool,

    pub hardware_id_present: bool,
    pub platform: String,
    pub model_count: usize,
    pub certified_model_count: usize,
    pub eligible_capability_count: usize,
    pub concurrency_limit: u16,

    pub unified_protocol_valid: bool,
    pub network_request_sent: bool,
}

impl ProductionHeartbeatClient {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            backend: BackendClient::new()?,
        })
    }

    pub fn send_live(
        &self,
        heartbeat: &ProductionHeartbeatV1,
    ) -> Result<ProductionHeartbeatSendResultV1, String> {
        let auth =
            SupabaseAuthClient::from_env()?;

        let ensured =
            auth.ensure_valid_session(true)?;

        let headers =
            BackendClient::authenticated_headers(
                &ensured.session
            )?;

        let mut response =
            self.backend
                .http()
                .post(self.backend.heartbeat_url())
                .headers(headers)
                .json(heartbeat)
                .send()
                .map_err(|_| {
                    "heartbeat_request_failed"
                        .to_string()
                })?;

        let mut auth_refresh_attempted = false;

        if response.status().as_u16() == 401 {
            auth_refresh_attempted = true;

            let refreshed =
                auth.force_refresh_session()?;

            let headers =
                BackendClient::authenticated_headers(
                    &refreshed.session
                )?;

            response =
                self.backend
                    .http()
                    .post(
                        self.backend.heartbeat_url()
                    )
                    .headers(headers)
                    .json(heartbeat)
                    .send()
                    .map_err(|_| {
                        "heartbeat_retry_failed"
                            .to_string()
                    })?;
        }

        let status_code =
            response.status().as_u16();

        if status_code == 401 {
            return Err(
                "heartbeat_auth_rejected_after_refresh"
                    .into()
            );
        }

        if status_code == 426 {
            return Ok(
                ProductionHeartbeatSendResultV1 {
                    status_code,
                    auth_refresh_attempted,
                    network_request_sent: true,
                    heartbeat_sent: false,
                    update_required: true,
                }
            );
        }

        if !response.status().is_success() {
            return Err(format!(
                "heartbeat_http_{}",
                status_code
            ));
        }

        Ok(ProductionHeartbeatSendResultV1 {
            status_code,
            auth_refresh_attempted,
            network_request_sent: true,
            heartbeat_sent: true,
            update_required: false,
        })
    }

    pub fn dry_run(
        &self,
        heartbeat: &ProductionHeartbeatV1,
    ) -> Result<ProductionHeartbeatDryRunV1, String> {
        let session =
            AuthSession::load_default()?;

        let headers =
            BackendClient::authenticated_headers(
                &session
            )?;

        let certified_model_count =
            heartbeat
                .metadata
                .model_capacity_v1
                .iter()
                .filter(|model| {
                    model.certified_concurrency
                        .unwrap_or(0) > 0
                        && format!(
                            "{:?}",
                            model.capacity_status
                        )
                        .eq_ignore_ascii_case(
                            "Certified"
                        )
                })
                .count();

        let unified_protocol_valid =
            heartbeat
                .metadata
                .unified_protocol_version
                == "edgeswarm-unified-heartbeat-v1";

        if heartbeat.hardware_id.trim().is_empty() {
            return Err(
                "heartbeat_hardware_id_missing"
                    .into()
            );
        }

        if !unified_protocol_valid {
            return Err(
                "heartbeat_unified_protocol_invalid"
                    .into()
            );
        }

        Ok(ProductionHeartbeatDryRunV1 {
            heartbeat_url:
                self.backend.heartbeat_url(),

            auth_session_valid:
                session.is_valid_now(),

            provider_email_present:
                session.provider_email().is_some(),

            bearer_header_ready:
                headers.contains_key(
                    reqwest::header::AUTHORIZATION
                ),

            hardware_id_present: true,

            platform:
                heartbeat.platform.clone(),

            model_count:
                heartbeat
                    .metadata
                    .model_capacity_v1
                    .len(),

            certified_model_count,

            eligible_capability_count:
                heartbeat
                    .eligible_model_capabilities
                    .len(),

            concurrency_limit:
                heartbeat.concurrency_limit,

            unified_protocol_valid,

            network_request_sent: false,
        })
    }
}
