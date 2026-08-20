pub mod auth_client;
pub mod auth_login_client;
pub mod auth_login_contract;
pub mod auth_session;
pub mod backend_client;
pub mod capacity;
pub mod capacity_policy;
pub mod capacity_store;
pub mod certificate_match;
pub mod certification_runner;
pub mod certification_workload;
pub mod generation_policy;
pub mod hardware;
pub mod hardware_identity;
pub mod heartbeat;
pub mod identity;
pub mod model;
pub mod model_discovery;
pub mod model_fingerprint;
pub mod model_provisioning;
pub mod model_registry;
pub mod node_service;
pub mod per_model_state;
pub mod platform;
pub mod production_heartbeat;
pub mod production_heartbeat_client;
pub mod production_inference;
pub mod production_prompt;
pub mod production_task_http;
pub mod result_signing;
pub mod task_client;
pub mod task_state;
pub mod wallet_account;
pub mod wallet_client;
pub mod wallet_identity;
pub mod wallet_public_identity;
pub mod wallet_vault;
pub mod workload_validator;

use crate::adapters;
use capacity::CapacityState;
use hardware::{AccelerationInfo, HardwareInfo};
use hardware_identity::HardwareIdentity;
use identity::NodeIdentity;
use model::ModelState;
use platform::PlatformInfo;
use serde::Serialize;
use task_state::TaskState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeState {
    pub identity: NodeIdentity,
    pub hardware_identity: HardwareIdentity,
    pub platform: PlatformInfo,
    pub hardware: HardwareInfo,
    pub acceleration: AccelerationInfo,
    pub models: Vec<ModelState>,
    pub tasks: TaskState,
    pub capacity: CapacityState,
}

impl NodeState {
    pub fn detect() -> Self {
        let identity_path = adapters::identity_file_path();
        let identity = NodeIdentity::load_or_create(&identity_path)
            .expect("failed to initialize persistent EdgeSwarm identity");

        let hardware_identity =
            HardwareIdentity::detect().expect("failed to initialize EdgeSwarm hardware identity");

        let platform = PlatformInfo::detect();
        let hardware = HardwareInfo::detect();
        let acceleration = adapters::detect_acceleration();

        let (models, capacity) = match (
            crate::runtime::llama_process::resolve_model_root_v1(),
            crate::runtime::llama_process::resolve_llama_server_path_v1(),
        ) {
            (Ok(model_root), Ok(runtime_path)) => {
                match per_model_state::resolve_per_model_states(
                    &model_root,
                    &runtime_path,
                    &identity.installation_id,
                    &acceleration.backend,
                ) {
                    Ok(states) => {
                        let models: Vec<ModelState> = states.iter().map(ModelState::from).collect();

                        let certified: Vec<_> = states
                            .iter()
                            .filter(|state| {
                                state.capacity_status == capacity::CapacityStatus::Certified
                            })
                            .collect();

                        let capacity = if certified.is_empty() {
                            let revalidation_required = states.iter().any(|state| {
                                state.capacity_status
                                    == capacity::CapacityStatus::RevalidationRequired
                            });

                            CapacityState {
                                status: if revalidation_required {
                                    capacity::CapacityStatus::RevalidationRequired
                                } else {
                                    capacity::CapacityStatus::Uncertified
                                },
                                ..CapacityState::default()
                            }
                        } else {
                            let conservative_concurrency = certified
                                .iter()
                                .filter_map(|state| state.certified_concurrency)
                                .min()
                                .unwrap_or(1)
                                .max(1);

                            CapacityState {
                                certified_concurrency: conservative_concurrency,
                                burst_concurrency: None,
                                status: capacity::CapacityStatus::Certified,
                                baseline_tokens_per_second: None,
                                certified_tokens_per_second: None,
                                certificates: Vec::new(),
                            }
                        };

                        (models, capacity)
                    }

                    Err(_) => (Vec::new(), CapacityState::default()),
                }
            }

            _ => (Vec::new(), CapacityState::default()),
        };

        Self {
            identity,
            hardware_identity,
            platform,
            hardware,
            acceleration,
            models,
            tasks: TaskState::default(),
            capacity,
        }
    }
}

pub mod deterministic_executor;
pub mod execution_model;
