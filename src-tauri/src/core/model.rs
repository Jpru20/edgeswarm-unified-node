use crate::core::capacity::CapacityStatus;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelState {
    pub selected_model: String,
    pub model_id: String,
    pub capability: String,
    pub tier: u8,
    pub runtime: String,
    pub acceleration: String,
    pub status: String,
    pub capacity_status: CapacityStatus,
    pub certified_concurrency: Option<u16>,
}
