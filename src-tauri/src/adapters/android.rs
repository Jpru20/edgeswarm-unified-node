use crate::core::hardware::AccelerationInfo;

pub fn detect() -> AccelerationInfo {
    AccelerationInfo {
        backend: "unprobed".into(),
        device_name: None,
        vram_bytes: None,
        available: false,
        detection_status: "android_adapter_pending".into(),
    }
}


pub fn platform_name() -> &'static str {
    "android"
}

pub fn app_data_dir() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("EDGESWARM_DATA_DIR") {
        return std::path::PathBuf::from(path);
    }

    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".edgeswarm")
        .join("unified-node")
}

pub fn hardware_identity_material()
    -> Option<(String, String)>
{
    for property in ["ro.serialno", "ro.boot.serialno"] {
        let output = std::process::Command::new("getprop")
            .arg(property)
            .output()
            .ok()?;

        if output.status.success() {
            let value =
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_lowercase();

            if !value.is_empty() && value != "unknown" {
                return Some((
                    format!("android_{property}"),
                    value,
                ));
            }
        }
    }

    // If Android does not expose stable device material to the
    // app sandbox, HardwareIdentity uses the persisted fallback.
    None
}
