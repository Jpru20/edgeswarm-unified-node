use crate::{adapters, core::hardware_identity::HardwareIdentity};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletPublicIdentity {
    pub identity_version: u16,
    pub hardware_id: String,
    pub wallet_address: String,
}

impl WalletPublicIdentity {
    pub fn default_path() -> PathBuf {
        adapters::identity_file_path()
            .with_file_name("wallet_identity.json")
    }

    pub fn load_default() -> Result<Self, String> {
        let raw = fs::read_to_string(Self::default_path())
            .map_err(|_| "wallet_identity_read_failed".to_string())?;

        serde_json::from_str(&raw)
            .map_err(|_| "wallet_identity_parse_failed".to_string())
    }

    pub fn save_current(wallet_address: &str) -> Result<Self, String> {
        let hardware = HardwareIdentity::detect()?;
        let wallet = wallet_address.trim();

        if wallet.len() != 42
            || !wallet.starts_with("0x")
            || !wallet[2..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("wallet_address_invalid".into());
        }

        let identity = Self {
            identity_version: 1,
            hardware_id: hardware.hardware_id,
            wallet_address: wallet.to_string(),
        };

        let path = Self::default_path();
        let parent = path.parent()
            .ok_or_else(|| "wallet_identity_parent_missing".to_string())?;

        fs::create_dir_all(parent)
            .map_err(|_| "wallet_identity_directory_failed".to_string())?;

        let temp = path.with_extension("json.tmp");

        fs::write(
            &temp,
            serde_json::to_vec_pretty(&identity)
                .map_err(|_| "wallet_identity_encode_failed".to_string())?,
        )
        .map_err(|_| "wallet_identity_write_failed".to_string())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &temp,
                fs::Permissions::from_mode(0o600),
            )
            .map_err(|_| "wallet_identity_permission_failed".to_string())?;
        }

        fs::rename(&temp, &path)
            .map_err(|_| "wallet_identity_replace_failed".to_string())?;

        Ok(identity)
    }
}
