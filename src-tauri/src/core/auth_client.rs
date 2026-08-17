use crate::core::auth_session::{
    AuthSession,
    SupabaseRefreshResponse,
};
use reqwest::{
    blocking::Client,
    header::{HeaderValue, AUTHORIZATION},
};
use serde_json::json;
use std::{env, time::Duration};

#[derive(Debug)]
pub struct SupabaseAuthClient {
    supabase_url: String,
    anon_key: String,
    http: Client,
}

#[derive(Debug)]
pub struct AuthEnsureResult {
    pub session: AuthSession,
    pub refreshed: bool,
    pub network_request_sent: bool,
}

impl SupabaseAuthClient {
    pub fn from_env() -> Result<Self, String> {
        let supabase_url = env::var("SUPABASE_URL")
            .or_else(|_| {
                env::var("EDGESWARM_SUPABASE_URL")
            })
            .map_err(|_| {
                "supabase_url_missing".to_string()
            })?
            .trim_end_matches('/')
            .to_string();

        let anon_key = env::var("SUPABASE_ANON_KEY")
            .or_else(|_| {
                env::var(
                    "EDGESWARM_SUPABASE_ANON_KEY"
                )
            })
            .map_err(|_| {
                "supabase_anon_key_missing".to_string()
            })?;

        if supabase_url.is_empty()
            || anon_key.trim().is_empty()
        {
            return Err(
                "supabase_auth_config_missing".into()
            );
        }

        let http = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| {
                "supabase_http_client_build_failed"
                    .to_string()
            })?;

        Ok(Self {
            supabase_url,
            anon_key,
            http,
        })
    }

    pub fn ensure_valid_session(
        &self,
        allow_refresh_network: bool,
    ) -> Result<AuthEnsureResult, String> {
        let mut session =
            AuthSession::load_default()?;

        if session.is_valid_now() {
            return Ok(AuthEnsureResult {
                session,
                refreshed: false,
                network_request_sent: false,
            });
        }

        if !session.mfa_verified() {
            return Err(
                "auth_session_mfa_not_verified"
                    .into()
            );
        }

        if !allow_refresh_network {
            return Err(
                "auth_session_refresh_required_dry_run"
                    .into()
            );
        }

        self.refresh_session(&mut session)?;

        if !session.is_valid_now() {
            return Err(
                "auth_session_invalid_after_refresh"
                    .into()
            );
        }

        session.save_secure()?;

        Ok(AuthEnsureResult {
            session,
            refreshed: true,
            network_request_sent: true,
        })
    }

    pub fn force_refresh_session(
        &self,
    ) -> Result<AuthEnsureResult, String> {
        let mut session =
            AuthSession::load_default()?;

        if !session.mfa_verified() {
            return Err(
                "auth_session_mfa_not_verified"
                    .into()
            );
        }

        self.refresh_session(&mut session)?;

        if !session.is_valid_now() {
            return Err(
                "auth_session_invalid_after_refresh"
                    .into()
            );
        }

        session.save_secure()?;

        Ok(AuthEnsureResult {
            session,
            refreshed: true,
            network_request_sent: true,
        })
    }

    fn refresh_session(
        &self,
        session: &mut AuthSession,
    ) -> Result<(), String> {
        let refresh_token = session
            .refresh_token()
            .ok_or_else(|| {
                "auth_refresh_token_missing"
                    .to_string()
            })?
            .to_string();

        let mut auth_value =
            HeaderValue::from_str(
                &format!(
                    "Bearer {}",
                    self.anon_key
                )
            )
            .map_err(|_| {
                "supabase_auth_header_invalid"
                    .to_string()
            })?;

        auth_value.set_sensitive(true);

        let response = self
            .http
            .post(format!(
                "{}/auth/v1/token?grant_type=refresh_token",
                self.supabase_url
            ))
            .header("apikey", &self.anon_key)
            .header(AUTHORIZATION, auth_value)
            .json(&json!({
                "refresh_token": refresh_token
            }))
            .send()
            .map_err(|_| {
                "supabase_refresh_request_failed"
                    .to_string()
            })?;

        if !response.status().is_success() {
            return Err(format!(
                "supabase_refresh_http_{}",
                response.status().as_u16()
            ));
        }

        let refresh =
            response
                .json::<SupabaseRefreshResponse>()
                .map_err(|_| {
                    "supabase_refresh_response_invalid"
                        .to_string()
                })?;

        let now = std::time::SystemTime::now()
            .duration_since(
                std::time::UNIX_EPOCH
            )
            .map(|value| value.as_secs())
            .unwrap_or(0);

        session.apply_refresh_response(
            refresh,
            now,
        )
    }
}
