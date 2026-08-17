use crate::adapters;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareIdentity {
    pub hardware_id: String,
    pub identity_version: u16,
    pub source: String,
    pub persistence_status: String,
}

impl HardwareIdentity {
    pub fn detect() -> Result<Self, String> {
        if let Some((source, material)) =
            adapters::hardware_identity_material()
        {
            return Self::from_material(
                &source,
                &material,
            );
        }

        Self::load_or_create_fallback(
            &adapters::hardware_identity_fallback_path(),
        )
    }

    pub fn from_material(
        source: &str,
        material: &str,
    ) -> Result<Self, String> {
        let source = source.trim();
        let material = material.trim();

        if source.is_empty() || material.is_empty() {
            return Err(
                "hardware_identity_material_missing"
                    .into(),
            );
        }

        let canonical = format!(
            "edgeswarm-hardware-id-v1\n{source}\n{}",
            material.to_lowercase()
        );

        let digest =
            Sha256::digest(canonical.as_bytes());

        let hardware_id = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        if !Self::is_valid_hardware_id(
            &hardware_id,
        ) {
            return Err(
                "hardware_identity_hash_invalid".into()
            );
        }

        Ok(Self {
            hardware_id,
            identity_version: 1,
            source: source.to_string(),
            persistence_status:
                "derived".into(),
        })
    }

    fn load_or_create_fallback(
        path: &Path,
    ) -> Result<Self, String> {
        if path.exists() {
            let raw = fs::read_to_string(path)
                .map_err(|_| {
                    "hardware_identity_fallback_read_failed"
                        .to_string()
                })?;

            let mut stored: Self =
                serde_json::from_str(&raw)
                    .map_err(|_| {
                        "hardware_identity_fallback_parse_failed"
                            .to_string()
                    })?;

            if !Self::is_valid_hardware_id(
                &stored.hardware_id,
            ) {
                return Err(
                    "hardware_identity_fallback_invalid"
                        .into(),
                );
            }

            stored.persistence_status =
                "persisted".into();

            return Ok(stored);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| {
                    "hardware_identity_fallback_directory_failed"
                        .to_string()
                })?;
        }

        let seed = Uuid::new_v4().to_string();

        let mut identity =
            Self::from_material(
                "persistent_random_fallback",
                &seed,
            )?;

        identity.persistence_status =
            "created".into();

        let raw =
            serde_json::to_string_pretty(&identity)
                .map_err(|_| {
                    "hardware_identity_fallback_encode_failed"
                        .to_string()
                })?;

        fs::write(path, raw)
            .map_err(|_| {
                "hardware_identity_fallback_write_failed"
                    .to_string()
            })?;

        Ok(identity)
    }

    pub fn is_valid_hardware_id(
        value: &str,
    ) -> bool {
        value.len() == 64
            && value.bytes().all(|byte| {
                byte.is_ascii_digit()
                    || (b'a'..=b'f')
                        .contains(&byte)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_identity_is_stable_sha256() {
        let first =
            HardwareIdentity::from_material(
                "linux_test",
                "DEVICE-123",
            )
            .unwrap();

        let second =
            HardwareIdentity::from_material(
                "linux_test",
                "device-123",
            )
            .unwrap();

        assert_eq!(
            first.hardware_id,
            second.hardware_id
        );

        assert!(
            HardwareIdentity::is_valid_hardware_id(
                &first.hardware_id
            )
        );
    }

    #[test]
    fn fallback_identity_persists() {
        let root = std::env::temp_dir().join(
            format!(
                "edgeswarm-hardware-id-test-{}",
                std::process::id()
            ),
        );

        let _ = fs::remove_dir_all(&root);

        let path =
            root.join("hardware_identity.json");

        let first =
            HardwareIdentity::
                load_or_create_fallback(&path)
                .unwrap();

        let second =
            HardwareIdentity::
                load_or_create_fallback(&path)
                .unwrap();

        assert_eq!(
            first.hardware_id,
            second.hardware_id
        );

        let _ = fs::remove_dir_all(&root);
    }
}
