use crate::core::{
    capacity::CapacityStatus,
    NodeState,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs};

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionHeartbeatMetadataV1 {
    pub unified_protocol_version: String,
    pub release_channel: String,
    pub architecture: String,
    pub package_type: String,
    pub runtime_sha256: Option<String>,
    pub public_release_safe: bool,
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

    pub model_id: Option<String>,
    pub model_status: String,
    pub model_capability: Option<String>,
    pub runtime: Option<String>,
    pub runtime_acceleration: String,

    pub eligible_model_capabilities: Vec<String>,
    pub models_available: Vec<String>,

    pub metadata: ProductionHeartbeatMetadataV1,
}

impl ProductionHeartbeatV1 {
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

        let concurrency_limit =
            if ready_models.is_empty() {
                1
            } else {
                state
                    .capacity
                    .certified_concurrency
                    .max(1)
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

            capabilities:
                eligible_model_capabilities.clone(),

            status:
                "online".into(),

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

            model_id:
                primary.map(|model| {
                    model.selected_model.clone()
                }),

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
                state.acceleration.backend.clone(),

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

                model_capacity_v1,
            },
        }
    }
}
