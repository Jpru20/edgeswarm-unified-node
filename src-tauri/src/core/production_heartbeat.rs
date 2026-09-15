use crate::core::{
    capacity::CapacityStatus,
    NodeState,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    sync::OnceLock,
    time::Instant,
};
use sysinfo::Disks;

fn unified_runtime_sha256_v1() -> Option<String> {
    let path = std::env::current_exe().ok()?;
    let bytes = fs::read(path).ok()?;
    let digest = Sha256::digest(bytes);

    Some(
        digest.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn unified_architecture_v1() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }.to_string()
}


static HEARTBEAT_PROCESS_STARTED_V1: OnceLock<Instant> =
    OnceLock::new();

fn process_uptime_sec_v1() -> u64 {
    HEARTBEAT_PROCESS_STARTED_V1
        .get_or_init(Instant::now)
        .elapsed()
        .as_secs()
}

fn model_disk_free_gb_v1() -> u64 {
    let root =
        crate::core::model_provisioning::model_root_v1();

    let disks =
        Disks::new_with_refreshed_list();

    let matched =
        disks
            .list()
            .iter()
            .filter(|disk| {
                root.starts_with(
                    disk.mount_point()
                )
            })
            .max_by_key(|disk| {
                disk
                    .mount_point()
                    .components()
                    .count()
            });

    matched
        .map(|disk| {
            disk.available_space() /
                1024 /
                1024 /
                1024
        })
        .or_else(|| {
            disks
                .list()
                .iter()
                .map(|disk| {
                    disk.available_space() /
                        1024 /
                        1024 /
                        1024
                })
                .max()
        })
        .unwrap_or(0)
}

pub fn selected_model_size_gb_v1(
    selected_model: &str,
) -> Option<f64> {
    let root =
        crate::core::model_provisioning::model_root_v1();

    crate::core::model_discovery::discover_models(&root)
        .into_iter()
        .find(|model| {
            model.selected_model ==
                selected_model
        })
        .and_then(|model| {
            fs::metadata(model.path).ok()
        })
        .map(|metadata| {
            metadata.len() as f64 /
                1024.0 /
                1024.0 /
                1024.0
        })
}

fn gpu_vendor_v1(
    device_name: Option<&str>,
    acceleration_backend: &str,
) -> Option<String> {
    let name =
        device_name
            .unwrap_or("")
            .to_ascii_lowercase();

    let backend =
        acceleration_backend
            .to_ascii_lowercase();

    if name.contains("nvidia") ||
        backend.contains("cuda")
    {
        return Some("NVIDIA".into());
    }

    if name.contains("amd") ||
        name.contains("radeon")
    {
        return Some("AMD".into());
    }

    if name.contains("intel") {
        return Some("Intel".into());
    }

    if name.contains("apple") ||
        backend.contains("metal")
    {
        return Some("Apple".into());
    }

    None
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionModelCapacityV1 {
    pub selected_model: String,
    pub model_id: String,
    pub capability: String,
    pub tier: u8,
    pub status: String,
    pub capacity_status: CapacityStatus,
    pub certified_concurrency: Option<u16>,
}

// WINDOWS_UPDATE_LIFECYCLE_HEARTBEAT_V1
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionUpdateLifecycleV1 {
    pub schema_version: u8,
    pub phase: String,
    pub from_version: String,
    pub to_version: String,
    pub started_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

#[cfg(target_os = "windows")]
fn active_update_lifecycle_v1(
    current_version: &str,
) -> Option<ProductionUpdateLifecycleV1> {
    let exe = std::env::current_exe().ok()?;
    let install_dir = exe.parent()?;
    let path = install_dir.join("update-lifecycle.json");

    let file_metadata = std::fs::metadata(&path).ok()?;
    let modified = file_metadata.modified().ok()?;
    let age = modified.elapsed().ok()?;

    // Never allow an abandoned updater marker to keep the node
    // permanently in an updating state.
    if age.as_secs() > 20 * 60 {
        return None;
    }

    let raw = std::fs::read_to_string(&path).ok()?;

    // UPDATE_LIFECYCLE_BOM_TOLERANCE_V1
    // Windows PowerShell 5.1 can emit an UTF-8 BOM. Accept both forms
    // so update telemetry cannot silently disappear because of encoding.
    let raw =
        raw.trim_start_matches('\u{feff}');

    let value: serde_json::Value =
        serde_json::from_str(raw).ok()?;

    let phase = value
        .get("phase")?
        .as_str()?
        .trim()
        .to_ascii_lowercase();

    if !matches!(
        phase.as_str(),
        "preparing"
            | "downloading"
            | "verifying"
            | "updating"
            | "installing"
            | "restarting"
    ) {
        return None;
    }

    let from_version = value
        .get("fromVersion")?
        .as_str()?
        .trim()
        .to_string();

    let to_version = value
        .get("toVersion")?
        .as_str()?
        .trim()
        .to_string();

    if to_version
        .trim_start_matches('v')
        .eq_ignore_ascii_case(
            current_version.trim_start_matches('v')
        )
    {
        return None;
    }

    Some(ProductionUpdateLifecycleV1 {
        schema_version: value
            .get("schemaVersion")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u8,

        phase,

        from_version,
        to_version,

        started_at_unix_ms: value
            .get("startedAtUnixMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),

        updated_at_unix_ms: value
            .get("updatedAtUnixMs")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    })
}

#[cfg(not(target_os = "windows"))]
fn active_update_lifecycle_v1(
    _current_version: &str,
) -> Option<ProductionUpdateLifecycleV1> {
    None
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionHeartbeatMetadataV1 {
    pub unified_protocol_version: String,
    pub release_channel: String,
    pub architecture: String,
    pub package_type: String,
    pub runtime_sha256: Option<String>,
    pub public_release_safe: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_lifecycle: Option<ProductionUpdateLifecycleV1>,

    pub model_capacity_v1: Vec<ProductionModelCapacityV1>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionHeartbeatV1 {
    pub hardware_id: String,
    pub worker: Option<String>,
    pub node_type: String,
    pub platform: String,
    pub app_version: String,
    pub capabilities: Vec<String>,
    pub status: String,

    pub current_task_ids: Vec<String>,
    pub concurrency_limit: u16,

    pub cpu_name: String,
    pub ram_gb: f64,
    pub disk_free_gb: u64,
    pub uptime_sec: u64,

    pub gpu_vendor: Option<String>,
    pub gpu_name: Option<String>,
    pub gpu_memory_mb: Option<u64>,

    pub cuda_available: bool,
    pub vulkan_available: bool,
    pub metal_available: bool,

    pub model_id: Option<String>,
    pub model_size_gb: Option<f64>,
    pub model_status: String,
    pub model_capability: Option<String>,
    pub runtime: Option<String>,
    pub runtime_acceleration: String,

    pub eligible_model_capabilities: Vec<String>,
    pub models_available: Vec<String>,

    pub metadata: ProductionHeartbeatMetadataV1,
}

impl ProductionHeartbeatV1 {
    // LIVE_UPDATE_LIFECYCLE_REFRESH_V1
    //
    // Production node_service keeps a long-lived base heartbeat.
    // Update lifecycle is dynamic, so refresh it immediately before
    // every network send instead of only when the heartbeat is created.
    pub fn refresh_update_lifecycle_v1(&mut self) {
        let lifecycle =
            active_update_lifecycle_v1(
                &self.app_version
            );

        if let Some(value) = lifecycle {
            self.metadata.update_lifecycle =
                Some(value);

            self.status =
                "updating".into();
        } else {
            self.metadata.update_lifecycle =
                None;

            if self.status
                .eq_ignore_ascii_case("updating")
            {
                self.status =
                    "online".into();
            }
        }
    }

    pub fn from_node_state(
        state: &NodeState,
        app_version: &str,
        node_type: &str,
        current_task_ids: &[String],
    ) -> Self {
        let mut ready_models = state
            .models
            .iter()
            .filter(|model| {
                model.status == "ready"
                    && model.capacity_status
                        == CapacityStatus::Certified
                    && model.certified_concurrency
                        .unwrap_or(0) > 0
            })
            .collect::<Vec<_>>();

        ready_models.sort_by(|left, right| {
            right
                .tier
                .cmp(&left.tier)
                .then_with(|| {
                    left.selected_model
                        .cmp(&right.selected_model)
                })
        });

        let primary = ready_models.first().copied();

        let eligible_model_capabilities =
            ready_models
                .iter()
                .map(|model| model.capability.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();

        // Every unified node retains the deterministic baseline.
        // Neural capabilities are added only after real certification.
        let capabilities = [
            "Exact-Extraction",
            "Data-Scraper",
            "Distributed-Compute",
        ]
        .iter()
        .map(|capability| (*capability).to_string())
        .chain(eligible_model_capabilities.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

        let models_available = state
            .models
            .iter()
            .map(|model| model.selected_model.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let model_capacity_v1 = state
            .models
            .iter()
            .map(|model| ProductionModelCapacityV1 {
                selected_model:
                    model.selected_model.clone(),
                model_id:
                    model.model_id.clone(),
                capability:
                    model.capability.clone(),
                tier:
                    model.tier,
                status:
                    model.status.clone(),
                capacity_status:
                    model.capacity_status.clone(),
                certified_concurrency:
                    model.certified_concurrency,
            })
            .collect();

        // PRODUCTION_CERTIFIED_CONCURRENCY_V1
        // The production dispatcher now executes up to the
        // certified concurrency of the active primary model.
        let concurrency_limit =
            primary
                .and_then(|model| {
                    model.certified_concurrency
                })
                .unwrap_or(1)
                .max(1)
                .min(5);

        let acceleration_backend =
            state.acceleration.backend
                .to_ascii_lowercase();

        let primary_model_size_gb =
            primary.and_then(|model| {
                selected_model_size_gb_v1(
                    &model.selected_model
                )
            });

        let update_lifecycle =
            active_update_lifecycle_v1(app_version);

        let heartbeat_status =
            if update_lifecycle.is_some() {
                "updating".to_string()
            } else {
                "online".to_string()
            };

        Self {
            hardware_id:
                state.hardware_identity.hardware_id.clone(),

            worker: crate::core::wallet_public_identity::WalletPublicIdentity::load_default()
                .ok()
                .filter(|wallet| wallet.hardware_id.eq_ignore_ascii_case(&state.hardware_identity.hardware_id))
                .map(|wallet| wallet.wallet_address),

            node_type:
                node_type.to_string(),

            platform:
                state.platform.os.clone(),

            app_version:
                app_version.to_string(),

            capabilities,

            status:
                heartbeat_status,

            current_task_ids:
                current_task_ids.to_vec(),

            concurrency_limit,

            cpu_name:
                state.hardware.cpu_brand.clone(),

            ram_gb:
                state.hardware.total_memory_bytes as f64
                    / 1024.0
                    / 1024.0
                    / 1024.0,

            disk_free_gb:
                model_disk_free_gb_v1(),

            uptime_sec:
                process_uptime_sec_v1(),

            gpu_vendor:
                gpu_vendor_v1(
                    state.acceleration
                        .device_name
                        .as_deref(),
                    &state.acceleration.backend,
                ),

            gpu_name:
                state.acceleration.device_name.clone(),

            gpu_memory_mb:
                state.acceleration
                    .vram_bytes
                    .map(|bytes| bytes / 1024 / 1024),

            cuda_available:
                acceleration_backend.contains("cuda"),

            vulkan_available:
                acceleration_backend.contains("vulkan"),

            metal_available:
                acceleration_backend.contains("metal"),

            model_id:
                primary.map(|model| {
                    model.selected_model.clone()
                }),

            model_size_gb:
                primary_model_size_gb,

            model_status:
                if primary.is_some() {
                    "ready".into()
                } else if state.models.is_empty() {
                    "not_installed".into()
                } else {
                    "installed_uncertified".into()
                },

            model_capability:
                primary.map(|model| {
                    model.capability.clone()
                }),

            runtime:
                primary.map(|model| {
                    model.runtime.clone()
                }),

            runtime_acceleration:
                primary
                    .map(|model| model.acceleration.clone())
                    .unwrap_or_else(|| state.acceleration.backend.clone()),

            eligible_model_capabilities,

            models_available,

            metadata: ProductionHeartbeatMetadataV1 {
                unified_protocol_version:
                    "edgeswarm-unified-heartbeat-v1".into(),

                release_channel:
                    "unified_private_candidate".into(),

                architecture:
                    unified_architecture_v1(),

                package_type:
                    "unified_binary".into(),

                runtime_sha256:
                    unified_runtime_sha256_v1(),

                public_release_safe:
                    false,

                update_lifecycle,

                model_capacity_v1,
            },
        }
    }
}
