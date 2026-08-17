use crate::core::hardware::AccelerationInfo;
use sha2::{Digest, Sha256};

pub fn detect() -> AccelerationInfo {
    if let Ok(output) = std::process::Command::new("nvidia-smi")
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

                let name = parts
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());

                let memory_mib = parts
                    .next()
                    .and_then(|value| value.trim().parse::<u64>().ok());

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

pub fn hardware_identity_override() -> Option<(String, String)> {
    let script = r#"[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;$u=(Get-CimInstance Win32_ComputerSystemProduct -ErrorAction Stop).UUID.Trim();$p=(Get-CimInstance Win32_Processor -ErrorAction Stop | Select-Object -First 1).ProcessorId.Trim();if($u -and $p){Write-Output ($u+'_'+$p)}"#;

    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let material = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_string();

    let digest = Sha256::digest(material.as_bytes());
    let hardware_id = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    Some(("windows_legacy_uuid_processor_id".into(), hardware_id))
}

pub fn hardware_identity_material() -> Option<(String, String)> {
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
                return Some(("windows_machine_guid".into(), value.to_lowercase()));
            }
        }
    }

    None
}
