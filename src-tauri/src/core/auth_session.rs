use crate::adapters;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    env,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const LEGACY_LINUX_AUTH_FILE: &str =
    "/etc/edgeswarm-node-auth.json";

#[derive(Debug, Clone)]
pub struct AuthSession {
    path: PathBuf,
    data: Map<String, Value>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct SupabaseRefreshResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
    pub expires_at: Option<u64>,
}


#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSessionSummary {
    pub auth_file_exists: bool,
    pub provider_email_present: bool,
    pub mfa_verified: bool,
    pub access_token_present: bool,
    pub refresh_token_present: bool,
    pub expires_at: Option<u64>,
    pub valid_without_refresh: bool,
}

impl AuthSession {
    pub fn from_authenticated_session(
        provider_email: &str,
        access_token: &str,
        refresh_token: &str,
        expires_at: u64,
    ) -> Result<Self, String> {
        let email = provider_email.trim().to_lowercase();
        let access = access_token.trim();
        let refresh = refresh_token.trim();

        if email.is_empty() || access.is_empty() || refresh.is_empty() {
            return Err("authenticated_session_fields_missing".into());
        }

        let mut data = Map::new();
        data.insert("authFileVersion".into(), Value::String("edgeswarm_unified_auth_v1".into()));
        data.insert("providerEmail".into(), Value::String(email));
        data.insert("accessToken".into(), Value::String(access.to_string()));
        data.insert("refreshToken".into(), Value::String(refresh.to_string()));
        data.insert("expiresAt".into(), Value::from(expires_at));
        data.insert("mfaVerified".into(), Value::Bool(true));

        Ok(Self {
            path: Self::default_path(),
            data,
        })
    }

    pub fn default_path() -> PathBuf {
        if let Some(path) =
            env::var_os("EDGESWARM_AUTH_FILE")
        {
            return PathBuf::from(path);
        }

        adapters::app_data_dir()
            .join("auth_session.json")
    }

    pub fn load_default() -> Result<Self, String> {
        Self::load_from_path(Self::default_path())
    }

    pub fn load_from_path(
        path: impl AsRef<Path>,
    ) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();

        let raw = fs::read_to_string(&path)
            .map_err(|err| {
                format!(
                    "auth_session_read_failed:{}",
                    err.kind()
                )
            })?;

        let value: Value =
            serde_json::from_str(&raw)
                .map_err(|_| {
                    "auth_session_json_invalid"
                        .to_string()
                })?;

        let data = value
            .as_object()
            .cloned()
            .ok_or_else(|| {
                "auth_session_root_not_object"
                    .to_string()
            })?;

        Ok(Self { path, data })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn provider_email(&self) -> Option<&str> {
        self.data
            .get("providerEmail")
            .or_else(|| self.data.get("email"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn mfa_verified(&self) -> bool {
        self.data
            .get("mfaVerified")
            .and_then(Value::as_bool)
            == Some(true)
    }

    pub fn access_token(&self) -> Option<&str> {
        self.data
            .get("accessToken")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn refresh_token(&self) -> Option<&str> {
        self.data
            .get("refreshToken")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn expires_at(&self) -> Option<u64> {
        let value = self.data.get("expiresAt")?;

        if let Some(number) = value.as_u64() {
            return Some(number);
        }

        value
            .as_str()
            .and_then(|text| text.parse::<u64>().ok())
    }

    pub fn is_valid_at(&self, now_unix: u64) -> bool {
        if !self.mfa_verified()
            || self.access_token().is_none()
        {
            return false;
        }

        match self.expires_at() {
            Some(expires_at) => {
                expires_at > now_unix.saturating_add(60)
            }
            None => true,
        }
    }

    pub fn is_valid_now(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0);

        self.is_valid_at(now)
    }

    pub fn apply_refresh_response(
        &mut self,
        response: SupabaseRefreshResponse,
        now_unix: u64,
    ) -> Result<(), String> {
        let access_token =
            response.access_token.trim();

        if access_token.is_empty() {
            return Err(
                "refresh_access_token_missing".into()
            );
        }

        let refresh_token = response
            .refresh_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                self.refresh_token()
                    .map(str::to_string)
            })
            .ok_or_else(|| {
                "refresh_token_missing".to_string()
            })?;

        let expires_at =
            response.expires_at.or_else(|| {
                response.expires_in.map(|seconds| {
                    now_unix.saturating_add(seconds)
                })
            });

        self.data.insert(
            "accessToken".into(),
            Value::String(access_token.to_string()),
        );

        self.data.insert(
            "refreshToken".into(),
            Value::String(refresh_token),
        );

        if let Some(expires_at) = expires_at {
            self.data.insert(
                "expiresAt".into(),
                Value::from(expires_at),
            );
        }

        self.data.insert(
            "mfaVerified".into(),
            Value::Bool(true),
        );

        self.data.insert(
            "lastRefreshAt".into(),
            Value::from(now_unix),
        );

        Ok(())
    }

    pub fn save_secure(&self) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| {
                "auth_session_parent_missing"
                    .to_string()
            })?;

        fs::create_dir_all(parent)
            .map_err(|_| {
                "auth_session_directory_create_failed"
                    .to_string()
            })?;

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);

        let temp_path = parent.join(
            format!(
                ".auth_session.{}.{}.tmp",
                std::process::id(),
                nonce
            )
        );

        let encoded =
            serde_json::to_vec_pretty(
                &Value::Object(self.data.clone())
            )
            .map_err(|_| {
                "auth_session_encode_failed"
                    .to_string()
            })?;

        let result = (|| -> Result<(), String> {
            let mut file =
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp_path)
                    .map_err(|_| {
                        "auth_session_temp_create_failed"
                            .to_string()
                    })?;

            file.write_all(&encoded)
                .map_err(|_| {
                    "auth_session_temp_write_failed"
                        .to_string()
                })?;

            file.write_all(b"\n")
                .map_err(|_| {
                    "auth_session_temp_write_failed"
                        .to_string()
                })?;

            file.sync_all()
                .map_err(|_| {
                    "auth_session_temp_sync_failed"
                        .to_string()
                })?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                fs::set_permissions(
                    &temp_path,
                    fs::Permissions::from_mode(0o600),
                )
                .map_err(|_| {
                    "auth_session_permission_failed"
                        .to_string()
                })?;
            }

            fs::rename(&temp_path, &self.path)
                .map_err(|_| {
                    "auth_session_replace_failed"
                        .to_string()
                })?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                fs::set_permissions(
                    &self.path,
                    fs::Permissions::from_mode(0o600),
                )
                .map_err(|_| {
                    "auth_session_permission_failed"
                        .to_string()
                })?;
            }

            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }

        result
    }

    pub fn summary(&self) -> AuthSessionSummary {
        AuthSessionSummary {
            auth_file_exists: self.path.is_file(),
            provider_email_present:
                self.provider_email().is_some(),
            mfa_verified:
                self.mfa_verified(),
            access_token_present:
                self.access_token().is_some(),
            refresh_token_present:
                self.refresh_token().is_some(),
            expires_at:
                self.expires_at(),
            valid_without_refresh:
                self.is_valid_now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_rotation_updates_and_persists_safely() {
        let root = std::env::temp_dir().join(
            format!(
                "edgeswarm-auth-test-{}",
                std::process::id()
            )
        );

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let path = root.join("auth_session.json");

        let mut data = Map::new();
        data.insert(
            "providerEmail".into(),
            Value::String("test@example.com".into()),
        );
        data.insert(
            "mfaVerified".into(),
            Value::Bool(true),
        );
        data.insert(
            "accessToken".into(),
            Value::String("old-access".into()),
        );
        data.insert(
            "refreshToken".into(),
            Value::String("old-refresh".into()),
        );
        data.insert(
            "expiresAt".into(),
            Value::from(100_u64),
        );

        let mut session = AuthSession {
            path: path.clone(),
            data,
        };

        session
            .apply_refresh_response(
                SupabaseRefreshResponse {
                    access_token:
                        "new-access".into(),
                    refresh_token:
                        Some("new-refresh".into()),
                    expires_in:
                        Some(3600),
                    expires_at:
                        None,
                },
                1000,
            )
            .unwrap();

        assert_eq!(
            session.access_token(),
            Some("new-access")
        );
        assert_eq!(
            session.refresh_token(),
            Some("new-refresh")
        );
        assert_eq!(
            session.expires_at(),
            Some(4600)
        );

        session.save_secure().unwrap();

        let reloaded =
            AuthSession::load_from_path(&path)
                .unwrap();

        assert_eq!(
            reloaded.access_token(),
            Some("new-access")
        );
        assert_eq!(
            reloaded.refresh_token(),
            Some("new-refresh")
        );
        assert_eq!(
            reloaded.expires_at(),
            Some(4600)
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn validates_with_same_sixty_second_margin() {
        let mut data = Map::new();

        data.insert(
            "mfaVerified".into(),
            Value::Bool(true),
        );
        data.insert(
            "accessToken".into(),
            Value::String("secret".into()),
        );
        data.insert(
            "refreshToken".into(),
            Value::String("refresh".into()),
        );
        data.insert(
            "expiresAt".into(),
            Value::from(1000_u64),
        );

        let session = AuthSession {
            path: PathBuf::from("/tmp/test"),
            data,
        };

        assert!(session.is_valid_at(900));
        assert!(!session.is_valid_at(940));
    }
}
