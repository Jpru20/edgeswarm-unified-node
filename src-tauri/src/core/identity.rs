use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeIdentity {
    pub installation_id: String,
    pub identity_version: u16,
    pub created_at_unix_ms: u128,
    pub persistence_status: String,
}

impl NodeIdentity {
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if path.exists() {
            let raw = fs::read_to_string(path)
                .map_err(|e| format!("identity_read_failed:{e}"))?;

            let mut identity: Self = serde_json::from_str(&raw)
                .map_err(|e| format!("identity_parse_failed:{e}"))?;

            identity.persistence_status = "persisted".into();
            return Ok(identity);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("identity_directory_failed:{e}"))?;
        }

        let created_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("identity_clock_failed:{e}"))?
            .as_millis();

        let identity = Self {
            installation_id: Uuid::new_v4().to_string(),
            identity_version: 1,
            created_at_unix_ms,
            persistence_status: "created".into(),
        };

        let raw = serde_json::to_string_pretty(&identity)
            .map_err(|e| format!("identity_serialize_failed:{e}"))?;

        fs::write(path, raw)
            .map_err(|e| format!("identity_write_failed:{e}"))?;

        Ok(identity)
    }
}
