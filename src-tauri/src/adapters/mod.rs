use crate::core::hardware::AccelerationInfo;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "android")]
mod android;

pub fn platform_name() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        return linux::platform_name();
    }

    #[cfg(target_os = "windows")]
    {
        return windows::platform_name();
    }

    #[cfg(target_os = "macos")]
    {
        return macos::platform_name();
    }

    #[cfg(target_os = "android")]
    {
        return android::platform_name();
    }

    #[allow(unreachable_code)]
    "unknown"
}

pub fn app_data_dir() -> std::path::PathBuf {
    #[cfg(target_os = "linux")]
    {
        return linux::app_data_dir();
    }

    #[cfg(target_os = "windows")]
    {
        return windows::app_data_dir();
    }

    #[cfg(target_os = "macos")]
    {
        return macos::app_data_dir();
    }

    #[cfg(target_os = "android")]
    {
        return android::app_data_dir();
    }

    #[allow(unreachable_code)]
    std::path::PathBuf::from(".")
}

pub fn identity_file_path() -> std::path::PathBuf {
    app_data_dir().join("node_identity.json")
}

pub fn hardware_identity_override() -> Option<(String, String)> {
    #[cfg(target_os = "windows")]
    {
        return windows::hardware_identity_override();
    }

    #[allow(unreachable_code)]
    None
}

pub fn hardware_identity_material() -> Option<(String, String)> {
    #[cfg(target_os = "linux")]
    {
        return linux::hardware_identity_material();
    }

    #[cfg(target_os = "windows")]
    {
        return windows::hardware_identity_material();
    }

    #[cfg(target_os = "macos")]
    {
        return macos::hardware_identity_material();
    }

    #[cfg(target_os = "android")]
    {
        return android::hardware_identity_material();
    }

    #[allow(unreachable_code)]
    None
}

pub fn hardware_identity_fallback_path() -> std::path::PathBuf {
    app_data_dir().join("hardware_identity.json")
}

pub fn detect_acceleration() -> AccelerationInfo {
    #[cfg(target_os = "linux")]
    {
        return linux::detect();
    }

    #[cfg(target_os = "windows")]
    {
        return windows::detect();
    }

    #[cfg(target_os = "macos")]
    {
        return macos::detect();
    }

    #[cfg(target_os = "android")]
    {
        return android::detect();
    }

    #[allow(unreachable_code)]
    AccelerationInfo {
        backend: "unknown".into(),
        device_name: None,
        vram_bytes: None,
        available: false,
        detection_status: "unsupported_platform".into(),
    }
}
