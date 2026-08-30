use crate::core::auth_login_contract::{
    AuthUser,
    MfaChallengeResponse,
    MfaVerifyResponse,
    PasswordAuthResponse,
};
use reqwest::{
    blocking::Client,
    header::{HeaderValue, AUTHORIZATION},
};
use serde_json::json;
use std::{env, time::Duration};

#[derive(Debug)]
pub struct SupabaseLoginClient {
    supabase_url: String,
    anon_key: String,
    http: Client,
}

#[derive(Debug)]
pub struct LoginTransportDryRun {
    pub password_url: String,
    pub user_url: String,
    pub challenge_template: String,
    pub verify_template: String,
    pub anon_key_present: bool,
    pub network_request_sent: bool,
}

impl SupabaseLoginClient {
    pub fn from_env() -> Result<Self, String> {
        let supabase_url = crate::core::production_config::supabase_url_v1()?;

        let anon_key = crate::core::production_config::supabase_anon_key_v1()?;

        if supabase_url.is_empty() || anon_key.trim().is_empty() {
            return Err("supabase_login_config_missing".into());
        }

        let http = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| "supabase_login_http_client_failed".to_string())?;

        Ok(Self {
            supabase_url,
            anon_key,
            http,
        })
    }

    fn bearer(value: &str) -> Result<HeaderValue, String> {
        let mut header = HeaderValue::from_str(
            &format!("Bearer {}", value.trim())
        )
        .map_err(|_| "supabase_bearer_header_invalid".to_string())?;

        header.set_sensitive(true);
        Ok(header)
    }

    pub fn password_login(
        &self,
        email: &str,
        password: &str,
    ) -> Result<PasswordAuthResponse, String> {
        if email.trim().is_empty() || password.is_empty() {
            return Err("login_credentials_missing".into());
        }

        let response = self.http
            .post(format!(
                "{}/auth/v1/token?grant_type=password",
                self.supabase_url
            ))
            .header("apikey", &self.anon_key)
            .header(
                AUTHORIZATION,
                Self::bearer(&self.anon_key)?,
            )
            .json(&json!({
                "email": email.trim().to_lowercase(),
                "password": password,
                "data": {},
                "gotrue_meta_security": {
                    "captcha_token": null
                }
            }))
            .send()
            .map_err(|_| "password_login_request_failed".to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "password_login_http_{}",
                response.status().as_u16()
            ));
        }

        response
            .json::<PasswordAuthResponse>()
            .map_err(|_| "password_login_response_invalid".to_string())
    }

    pub fn get_user(
        &self,
        access_token: &str,
    ) -> Result<AuthUser, String> {
        let response = self.http
            .get(format!("{}/auth/v1/user", self.supabase_url))
            .header("apikey", &self.anon_key)
            .header(
                AUTHORIZATION,
                Self::bearer(access_token)?,
            )
            .send()
            .map_err(|_| "get_user_request_failed".to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "get_user_http_{}",
                response.status().as_u16()
            ));
        }

        response
            .json::<AuthUser>()
            .map_err(|_| "get_user_response_invalid".to_string())
    }

    pub fn challenge(
        &self,
        access_token: &str,
        factor_id: &str,
    ) -> Result<MfaChallengeResponse, String> {
        let response = self.http
            .post(format!(
                "{}/auth/v1/factors/{}/challenge",
                self.supabase_url,
                factor_id.trim()
            ))
            .header("apikey", &self.anon_key)
            .header(
                AUTHORIZATION,
                Self::bearer(access_token)?,
            )
            .json(&json!({"channel": null}))
            .send()
            .map_err(|_| "mfa_challenge_request_failed".to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "mfa_challenge_http_{}",
                response.status().as_u16()
            ));
        }

        response
            .json::<MfaChallengeResponse>()
            .map_err(|_| "mfa_challenge_response_invalid".to_string())
    }

    pub fn verify(
        &self,
        access_token: &str,
        factor_id: &str,
        challenge_id: &str,
        code: &str,
    ) -> Result<MfaVerifyResponse, String> {
        let response = self.http
            .post(format!(
                "{}/auth/v1/factors/{}/verify",
                self.supabase_url,
                factor_id.trim()
            ))
            .header("apikey", &self.anon_key)
            .header(
                AUTHORIZATION,
                Self::bearer(access_token)?,
            )
            .json(&json!({
                "factor_id": factor_id.trim(),
                "challenge_id": challenge_id.trim(),
                "code": code.trim()
            }))
            .send()
            .map_err(|_| "mfa_verify_request_failed".to_string())?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().unwrap_or_default();

            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| {
                    value.get("error_code")
                        .or_else(|| value.get("code"))
                        .or_else(|| value.get("msg"))
                        .or_else(|| value.get("message"))
                        .and_then(|item| item.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "verification_rejected".to_string());

            return Err(format!(
                "mfa_verify_http_{status}:{detail}"
            ));
        }

        response
            .json::<MfaVerifyResponse>()
            .map_err(|_| "mfa_verify_response_invalid".to_string())
    }

    pub fn dry_run(&self) -> LoginTransportDryRun {
        LoginTransportDryRun {
            password_url: format!(
                "{}/auth/v1/token?grant_type=password",
                self.supabase_url
            ),
            user_url: format!(
                "{}/auth/v1/user",
                self.supabase_url
            ),
            challenge_template: format!(
                "{}/auth/v1/factors/{{factorId}}/challenge",
                self.supabase_url
            ),
            verify_template: format!(
                "{}/auth/v1/factors/{{factorId}}/verify",
                self.supabase_url
            ),
            anon_key_present: !self.anon_key.trim().is_empty(),
            network_request_sent: false,
        }
    }
}
