use crate::core::certification_workload::CertificationProfile;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CapacityStatus {
    Uncertified,
    Certified,
    RevalidationRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityTaskResult {
    pub workload_id: String,
    pub profile: CertificationProfile,
    pub success: bool,
    pub output_valid: bool,
    pub wall_time_ms: u64,
    pub first_token_ms: Option<u64>,
    pub output_tokens: u32,
    pub tokens_per_second: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityBenchmarkSample {
    pub certification_pack_id: String,
    pub concurrency: u16,
    pub wall_time_ms: u64,
    pub median_task_wall_time_ms: u64,
    pub median_first_token_ms: Option<u64>,
    pub aggregate_tokens_per_second: f64,
    pub peak_memory_bytes: Option<u64>,
    pub thermal_throttled: bool,
    pub successful_tasks: u16,
    pub failed_tasks: u16,
    pub valid_outputs: u16,
    pub quality_pass_rate: f64,
    pub task_results: Vec<CapacityTaskResult>,
}

fn default_output_limit_policy_version() -> String {
    "legacy-unversioned".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityCertificateV1 {
    pub certificate_version: String,
    pub certification_pack_id: String,
    pub installation_id: String,
    pub model_id: String,
    pub model_sha256: String,
    pub model_capability: String,
    pub quantization: String,
    pub runtime: String,
    pub runtime_version: String,
    pub acceleration: String,
    pub benchmark_mode: String,
    pub capacity_policy_version: String,
    #[serde(default = "default_output_limit_policy_version")]
    pub output_limit_policy_version: String,
    pub tested_concurrency_levels: Vec<u16>,
    pub rejected_concurrency: Option<u16>,
    pub certified_concurrency: u16,
    pub burst_concurrency: Option<u16>,
    pub baseline_tokens_per_second: f64,
    pub certified_tokens_per_second: f64,
    pub latency_multiplier: f64,
    pub quality_pass_rate: f64,
    pub app_version: String,
    pub created_at_unix_ms: u128,
    pub samples: Vec<CapacityBenchmarkSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityState {
    pub certified_concurrency: u16,
    pub burst_concurrency: Option<u16>,
    pub status: CapacityStatus,
    pub baseline_tokens_per_second: Option<f64>,
    pub certified_tokens_per_second: Option<f64>,
    pub certificates: Vec<CapacityCertificateV1>,
}

impl Default for CapacityState {
    fn default() -> Self {
        Self {
            certified_concurrency: 1,
            burst_concurrency: None,
            status: CapacityStatus::Uncertified,
            baseline_tokens_per_second: None,
            certified_tokens_per_second: None,
            certificates: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertified_capacity_stays_at_one_slot() {
        let state = CapacityState::default();

        assert_eq!(state.certified_concurrency, 1);
        assert_eq!(state.status, CapacityStatus::Uncertified);
        assert!(state.certificates.is_empty());
    }
}
