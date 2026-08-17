use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct AuthFactor {
    pub id: String,

    #[serde(rename = "factor_type")]
    pub factor_type: String,

    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthUser {
    pub email: Option<String>,

    #[serde(default)]
    pub factors: Vec<AuthFactor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PasswordAuthResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Option<u64>,
    pub expires_in: Option<u64>,
    pub user: Option<AuthUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MfaChallengeResponse {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MfaVerifyResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Option<u64>,
    pub expires_in: Option<u64>,
    pub user: Option<AuthUser>,
}

pub fn jwt_aal(access_token: &str) -> Option<String> {
    let payload = access_token.split(".").nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value.get("aal")?.as_str().map(str::to_string)
}

pub fn verified_totp_factor(
    user: &AuthUser,
) -> Option<&AuthFactor> {
    user.factors.iter().find(|factor| {
        factor.factor_type
            .eq_ignore_ascii_case("totp")
            && factor.status
                .eq_ignore_ascii_case("verified")
            && !factor.id.trim().is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_aal2_from_jwt_payload() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"aal":"aal2"}"#);
        let token = format!("header.{payload}.signature");
        assert_eq!(jwt_aal(&token).as_deref(), Some("aal2"));
    }

    #[test]
    fn parses_password_session() {
        let parsed: PasswordAuthResponse =
            serde_json::from_str(
                r#"{
                    "access_token":"a",
                    "refresh_token":"r",
                    "expires_in":3600,
                    "user":{"email":"test@example.com"}
                }"#,
            )
            .unwrap();

        assert_eq!(
            parsed.access_token,
            "a"
        );
        assert_eq!(
            parsed.refresh_token,
            "r"
        );
        assert_eq!(
            parsed.expires_in,
            Some(3600)
        );
    }

    #[test]
    fn selects_only_verified_totp() {
        let user: AuthUser =
            serde_json::from_str(
                r#"{
                    "email":"test@example.com",
                    "factors":[
                        {
                            "id":"phone-1",
                            "factor_type":"phone",
                            "status":"verified"
                        },
                        {
                            "id":"totp-old",
                            "factor_type":"totp",
                            "status":"unverified"
                        },
                        {
                            "id":"totp-good",
                            "factor_type":"totp",
                            "status":"verified"
                        }
                    ]
                }"#,
            )
            .unwrap();

        assert_eq!(
            verified_totp_factor(&user)
                .unwrap()
                .id,
            "totp-good"
        );
    }

    #[test]
    fn parses_mfa_verify_session() {
        let parsed: MfaVerifyResponse =
            serde_json::from_str(
                r#"{
                    "access_token":"aal2-a",
                    "refresh_token":"aal2-r",
                    "expires_at":2000,
                    "user":{"email":"test@example.com"}
                }"#,
            )
            .unwrap();

        assert_eq!(
            parsed.expires_at,
            Some(2000)
        );
    }
}
