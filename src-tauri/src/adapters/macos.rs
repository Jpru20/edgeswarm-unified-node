use crate::core::hardware::AccelerationInfo;

pub fn detect() -> AccelerationInfo {
    AccelerationInfo {
        backend: "unprobed".into(),
        device_name: None,
        vram_bytes: None,
        available: false,
        detection_status: "macos_adapter_pending".into(),
    }
}

pub fn platform_name() -> &'static str {
    "macos"
}

pub fn app_data_dir() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("EDGESWARM_DATA_DIR") {
        return std::path::PathBuf::from(path);
    }

    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Library")
        .join("Application Support")
        .join("EdgeSwarm")
        .join("unified-node")
}

fn legacy_device_scoped_hardware_id_v1(io_platform_uuid: &str) -> Option<String> {
    use sha2::{Digest, Sha256};

    let uuid = io_platform_uuid.trim().to_lowercase();

    if uuid.is_empty() {
        return None;
    }

    let material = format!(
        "{{\"identityVersion\":1,\"osType\":\"macos\",\"rawStableLocalId\":\"ioplatformuuid:{uuid}\"}}"
    );

    let digest = Sha256::digest(material.as_bytes());

    Some(
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    )
}

pub fn hardware_identity_override() -> Option<(String, String)> {
    let (_, io_platform_uuid) = hardware_identity_material()?;
    let hardware_id = legacy_device_scoped_hardware_id_v1(&io_platform_uuid)?;

    Some(("macos_legacy_v013_device_scoped".into(), hardware_id))
}

pub fn hardware_identity_material() -> Option<(String, String)> {
    let output = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);

    for line in text.lines() {
        if line.contains("IOPlatformUUID") {
            let value = line.split('=').nth(1)?.trim().trim_matches('"').trim();

            if !value.is_empty() {
                return Some(("macos_io_platform_uuid".into(), value.to_lowercase()));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_legacy_device_scoped_id_matches_v013() {
        let hardware_id =
            legacy_device_scoped_hardware_id_v1("4B5DED3D-4BA9-510F-9F9A-7559EFE9102E").unwrap();

        assert_eq!(
            hardware_id,
            "66fb1203c40822d014827ede7b200e5afafd03043e474a61a57720618b27f50b"
        );
    }
}
