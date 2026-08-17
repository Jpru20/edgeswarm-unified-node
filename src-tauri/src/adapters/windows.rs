use crate::core::hardware::AccelerationInfo;

pub fn detect() -> AccelerationInfo {
    AccelerationInfo {
        backend: "unprobed".into(),
        device_name: None,
        vram_bytes: None,
        available: false,
        detection_status: "windows_adapter_pending".into(),
    }
}


pub fn platform_name() -> &'static str {
    "windows"
}

pub fn app_data_dir() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("EDGESWARM_DATA_DIR") {
        return std::path::PathBuf::from(path);
    }

    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("EdgeSwarm")
        .join("unified-node")
}

pub fn hardware_identity_material()
    -> Option<(String, String)>
{
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);

    for line in text.lines() {
        if line.to_ascii_lowercase().contains("machineguid") {
            let value = line.split_whitespace().last()?.trim();

            if !value.is_empty() {
                return Some((
                    "windows_machine_guid".into(),
                    value.to_lowercase(),
                ));
            }
        }
    }

    None
}
