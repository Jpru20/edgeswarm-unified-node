use crate::core::hardware::AccelerationInfo;
use std::process::Command;

pub fn detect() -> AccelerationInfo {
    if let Ok(output) = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);

            if let Some(line) = text.lines().next() {
                let mut parts = line.splitn(2, ',');
                let name = parts.next().map(str::trim).filter(|v| !v.is_empty());
                let memory_mib = parts
                    .next()
                    .and_then(|v| v.trim().parse::<u64>().ok());

                return AccelerationInfo {
                    backend: "cuda".into(),
                    device_name: name.map(str::to_string),
                    vram_bytes: memory_mib.map(|mib| mib * 1024 * 1024),
                    available: true,
                    detection_status: "detected".into(),
                };
            }
        }
    }

    AccelerationInfo {
        backend: "cpu".into(),
        device_name: None,
        vram_bytes: None,
        available: true,
        detection_status: "cpu_fallback".into(),
    }
}

pub fn platform_name() -> &'static str {
    "linux"
}

pub fn app_data_dir() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("EDGESWARM_DATA_DIR") {
        return std::path::PathBuf::from(path);
    }

    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return std::path::PathBuf::from(xdg)
            .join("edgeswarm")
            .join("unified-node");
    }

    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    home.join(".local")
        .join("share")
        .join("edgeswarm")
        .join("unified-node")
}


fn stable_identity_value(path: &str) -> Option<String> {
    let value = std::fs::read_to_string(path)
        .ok()?
        .trim()
        .to_lowercase();

    if value.is_empty() {
        return None;
    }

    let invalid = [
        "none",
        "unknown",
        "not specified",
        "default string",
        "to be filled by o.e.m.",
        "00000000-0000-0000-0000-000000000000",
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
    ];

    if invalid.contains(&value.as_str()) {
        return None;
    }

    Some(value)
}

pub fn hardware_identity_material()
    -> Option<(String, String)>
{
    let candidates = [
        (
            "linux_dmi_product_uuid",
            "/sys/class/dmi/id/product_uuid",
        ),
        (
            "linux_dmi_board_serial",
            "/sys/class/dmi/id/board_serial",
        ),
        (
            "linux_dmi_product_serial",
            "/sys/class/dmi/id/product_serial",
        ),
        (
            "linux_machine_id",
            "/etc/machine-id",
        ),
    ];

    for (source, path) in candidates {
        if let Some(value) =
            stable_identity_value(path)
        {
            return Some((
                source.to_string(),
                value,
            ));
        }
    }

    None
}
