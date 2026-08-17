use crate::core::{
    capacity::CapacityStatus,
    hardware::{AccelerationInfo, HardwareInfo},
    model::ModelState,
    platform::PlatformInfo,
    NodeState,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedHeartbeatV1 {
    pub protocol_version: String,
    pub app_version: String,
    pub installation_id: String,
    pub node_type: String,
    pub status: String,
    pub platform: PlatformInfo,
    pub hardware: HardwareInfo,
    pub acceleration: AccelerationInfo,
    pub models: Vec<ModelState>,
    pub active_task_ids: Vec<String>,
    pub concurrency_limit: u16,
    pub capacity_status: CapacityStatus,
}

impl UnifiedHeartbeatV1 {
    pub fn from_state(state: &NodeState) -> Self {
        let concurrency_limit = match state.capacity.status {
            CapacityStatus::Certified => {
                state.capacity.certified_concurrency.max(1)
            }
            _ => 1,
        };

        Self {
            protocol_version: "edgeswarm-unified-heartbeat-v1".into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            installation_id: state.identity.installation_id.clone(),
            node_type: "unified".into(),
            status: "online".into(),
            platform: state.platform.clone(),
            hardware: state.hardware.clone(),
            acceleration: state.acceleration.clone(),
            models: state.models.clone(),
            active_task_ids: state.tasks.active_task_ids.clone(),
            concurrency_limit,
            capacity_status: state.capacity.status.clone(),
        }
    }
}
