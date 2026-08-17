use serde::Serialize;
use sysinfo::System;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    pub logical_cpu_count: usize,
    pub physical_cpu_count: Option<usize>,
    pub cpu_brand: String,
    pub cpu_vendor: String,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
}

impl HardwareInfo {
    pub fn detect() -> Self {
        let system = System::new_all();

        let fallback_logical = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1);

        let logical_cpu_count = if system.cpus().is_empty() {
            fallback_logical
        } else {
            system.cpus().len()
        };

        let cpu_brand = system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_string());

        let cpu_vendor = system
            .cpus()
            .first()
            .map(|cpu| cpu.vendor_id().trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_string());

        Self {
            logical_cpu_count,
            physical_cpu_count: System::physical_core_count(),
            cpu_brand,
            cpu_vendor,
            total_memory_bytes: system.total_memory(),
            available_memory_bytes: system.available_memory(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccelerationInfo {
    pub backend: String,
    pub device_name: Option<String>,
    pub vram_bytes: Option<u64>,
    pub available: bool,
    pub detection_status: String,
}
