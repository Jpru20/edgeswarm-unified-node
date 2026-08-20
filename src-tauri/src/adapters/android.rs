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

pub fn hardware_identity_material() -> Option<(String, String)> {
    // ANDROID_UNIFIED_HARDWARE_IDENTITY_BRIDGE_PENDING_V1
    //
    // The unified Android host must provide the existing Android
    // device-scoped material:
    //
    //   ${Build.MANUFACTURER}_${Build.MODEL} + "_" +
    //   Settings.Secure.ANDROID_ID.take(6)
    //
    // with source:
    //
    //   android_legacy_device_scoped_v1
    //
    // Rust then owns the canonical v1 SHA-256 derivation.
    //
    // Do not use ro.serialno, ro.boot.serialno, or another device
    // identifier here because that would fork an already-migrated
    // Android node into a second hardware identity.
    None
}
