use crate::core::wallet_identity::WorkerWalletRow;
use reqwest::{
    blocking::Client,
    header::{HeaderValue, AUTHORIZATION},
};
use serde_json::json;
use std::{env, time::Duration};

pub struct WorkerWalletClient {
    supabase_url: String,
    anon_key: String,
    http: Client,
}

impl WorkerWalletClient {
    pub fn from_env() -> Result<Self, String> {
        let supabase_url = crate::core::production_config::supabase_url_v1()?;

        let anon_key = crate::core::production_config::supabase_anon_key_v1()?;

        Ok(Self {
            supabase_url,
            anon_key,
            http: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|_| "wallet_http_client_failed".to_string())?,
        })
    }

    fn bearer(token: &str) -> Result<HeaderValue, String> {
        let mut value = HeaderValue::from_str(
            &format!("Bearer {}", token.trim())
        )
        .map_err(|_| "wallet_bearer_invalid".to_string())?;

        value.set_sensitive(true);
        Ok(value)
    }

    pub fn rows_for_email(
        &self,
        access_token: &str,
        email: &str,
    ) -> Result<Vec<WorkerWalletRow>, String> {
        let response = self.http
            .get(format!("{}/rest/v1/worker_wallets", self.supabase_url))
            .header("apikey", &self.anon_key)
            .header(AUTHORIZATION, Self::bearer(access_token)?)
            .query(&[
                ("email", format!("eq.{}", email.trim().to_lowercase())),
                ("select", "id,hardware_id,private_key".to_string()),
            ])
            .send()
            .map_err(|_| "wallet_lookup_request_failed".to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "wallet_lookup_http_{}",
                response.status().as_u16()
            ));
        }

        response
            .json::<Vec<WorkerWalletRow>>()
            .map_err(|_| "wallet_lookup_response_invalid".to_string())
    }

    pub fn insert_device(
        &self,
        access_token: &str,
        email: &str,
        hardware_id: &str,
        encrypted_private_key: &str,
    ) -> Result<u16, String> {
        let response = self.http
            .post(format!("{}/rest/v1/worker_wallets", self.supabase_url))
            .header("apikey", &self.anon_key)
            .header(AUTHORIZATION, Self::bearer(access_token)?)
            .json(&json!({
                "email": email.trim().to_lowercase(),
                "hardware_id": hardware_id,
                "private_key": encrypted_private_key
            }))
            .send()
            .map_err(|_| "wallet_insert_request_failed".to_string())?;

        Ok(response.status().as_u16())
    }
}
