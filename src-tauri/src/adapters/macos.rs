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

pub fn hardware_identity_material()
    -> Option<(String, String)>
{
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
            let value = line
                .split('=')
                .nth(1)?
                .trim()
                .trim_matches('"')
                .trim();

            if !value.is_empty() {
                return Some((
                    "macos_io_platform_uuid".into(),
                    value.to_lowercase(),
                ));
            }
        }
    }

    None
}
