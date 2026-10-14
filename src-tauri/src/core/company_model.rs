use crate::{
    adapters,
    core::{
        capacity::CapacityStatus,
        NodeState,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{
        SystemTime,
        UNIX_EPOCH,
    },
};

pub const COMPANY_MODEL_INFERENCE_CAPABILITY_V1: &str =
    "Company-Model-Inference-V1";

pub const COMPANY_MODEL_EXECUTION_CONTRACT_V1: &str =
    "company-model-execution-v1";

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub struct CompanyModelExecutionContractV1 {
    pub contract_version: String,
    pub company_model_id: String,
    pub company_model_version: u64,
    pub base_model_id: String,
    pub base_capability: String,
    pub adapter_sha256: String,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub struct CompanyModelDeploymentV1 {
    pub company_model_id: String,
    pub version: u64,
    pub base_model_id: String,
    pub base_capability: String,
    pub adapter_sha256: String,
    pub adapter_download_url: String,
    pub status: String,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub struct CompanyModelLocalStateV1 {
    pub schema_version: u8,
    pub company_model_id: String,
    pub version: u64,
    pub base_model_id: String,
    pub base_capability: String,
    pub adapter_sha256: String,
    pub adapter_path: String,
    pub status: String,
    pub updated_at_unix_ms: u64,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
)]
pub struct CompanyModelRuntimeKeyV1 {
    pub company_model_id: String,
    pub version: u64,
    pub base_model_id: String,
    pub adapter_sha256: String,
    pub adapter_path: PathBuf,
}

#[derive(
    Debug,
    Clone,
    Serialize,
)]
#[serde(rename_all = "camelCase")]
pub struct CompanyModelHeartbeatV1 {
    pub ready: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_model_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_model_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_capability: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_sha256: Option<String>,

    pub reason: String,
}

fn now_unix_ms_v1() -> u64 {
    SystemTime::now()
        .duration_since(
            UNIX_EPOCH
        )
        .unwrap_or_default()
        .as_millis()
        as u64
}

fn valid_sha256_v1(
    value: &str,
) -> bool {
    value.len() == 64 &&
        value
            .chars()
            .all(|c| c.is_ascii_hexdigit())
}

pub fn company_model_root_v1() -> PathBuf {
    adapters::app_data_dir()
        .join("company-model")
}

fn local_state_path_v1() -> PathBuf {
    company_model_root_v1()
        .join("state.json")
}

pub fn load_local_company_model_v1(
) -> Option<CompanyModelLocalStateV1> {
    let raw =
        fs::read_to_string(
            local_state_path_v1()
        )
        .ok()?;

    let state =
        serde_json::from_str::<
            CompanyModelLocalStateV1
        >(&raw)
        .ok()?;

    if state.schema_version != 1 ||
        state.company_model_id
            .trim()
            .is_empty() ||
        state.base_model_id
            .trim()
            .is_empty() ||
        !valid_sha256_v1(
            &state.adapter_sha256
        ) ||
        !state
            .status
            .eq_ignore_ascii_case(
                "ready"
            )
    {
        return None;
    }

    Some(state)
}

fn persist_local_state_v1(
    state: &CompanyModelLocalStateV1,
) -> Result<(), String> {
    let root =
        company_model_root_v1();

    fs::create_dir_all(
        &root
    )
    .map_err(|_| {
        "company_model_state_directory_failed"
            .to_string()
    })?;

    let target =
        local_state_path_v1();

    let temp =
        target.with_extension(
            "json.tmp"
        );

    fs::write(
        &temp,
        serde_json::to_vec_pretty(
            state
        )
        .map_err(|_| {
            "company_model_state_encode_failed"
                .to_string()
        })?,
    )
    .map_err(|_| {
        "company_model_state_write_failed"
            .to_string()
    })?;

    fs::rename(
        temp,
        target
    )
    .map_err(|_| {
        "company_model_state_commit_failed"
            .to_string()
    })?;

    Ok(())
}

fn certified_base_present_v1(
    state: &NodeState,
    base_model_id: &str,
) -> bool {
    state
        .models
        .iter()
        .any(|model| {
            model.selected_model
                .eq_ignore_ascii_case(
                    base_model_id
                ) &&
            model.status ==
                "ready" &&
            model.capacity_status ==
                CapacityStatus::Certified
        })
}

pub fn company_model_heartbeat_v1(
    state: &NodeState,
) -> CompanyModelHeartbeatV1 {
    let local =
        match load_local_company_model_v1() {
            Some(value) =>
                value,

            None => {
                return CompanyModelHeartbeatV1 {
                    ready: false,
                    company_model_id: None,
                    version: None,
                    base_model_id: None,
                    base_capability: None,
                    adapter_sha256: None,
                    reason:
                        "company_model_not_installed"
                            .into(),
                };
            }
        };

    if !certified_base_present_v1(
        state,
        &local.base_model_id
    ) {
        return CompanyModelHeartbeatV1 {
            ready: false,
            company_model_id:
                Some(
                    local.company_model_id
                ),
            version:
                Some(
                    local.version
                ),
            base_model_id:
                Some(
                    local.base_model_id
                ),
            base_capability:
                Some(
                    local.base_capability
                ),
            adapter_sha256:
                Some(
                    local.adapter_sha256
                ),
            reason:
                "company_model_base_not_certified"
                    .into(),
        };
    }

    if !Path::new(
        &local.adapter_path
    )
    .is_file()
    {
        return CompanyModelHeartbeatV1 {
            ready: false,
            company_model_id:
                Some(
                    local.company_model_id
                ),
            version:
                Some(
                    local.version
                ),
            base_model_id:
                Some(
                    local.base_model_id
                ),
            base_capability:
                Some(
                    local.base_capability
                ),
            adapter_sha256:
                Some(
                    local.adapter_sha256
                ),
            reason:
                "company_model_adapter_missing"
                    .into(),
        };
    }

    CompanyModelHeartbeatV1 {
        ready: true,
        company_model_id:
            Some(
                local.company_model_id
            ),
        version:
            Some(
                local.version
            ),
        base_model_id:
            Some(
                local.base_model_id
            ),
        base_capability:
            Some(
                local.base_capability
            ),
        adapter_sha256:
            Some(
                local.adapter_sha256
            ),
        reason:
            "company_model_ready"
                .into(),
    }
}

pub fn install_company_adapter_v1(
    deployment:
        &CompanyModelDeploymentV1,
    verified_temp_adapter:
        &Path,
) -> Result<
    CompanyModelLocalStateV1,
    String
> {
    if !deployment
        .status
        .eq_ignore_ascii_case(
            "certified"
        )
    {
        return Err(
            "company_model_deployment_not_certified"
                .into()
        );
    }

    if deployment
        .company_model_id
        .trim()
        .is_empty() ||
        deployment
            .base_model_id
            .trim()
            .is_empty() ||
        deployment.version == 0 ||
        !valid_sha256_v1(
            &deployment.adapter_sha256
        )
    {
        return Err(
            "company_model_deployment_invalid"
                .into()
        );
    }

    if !verified_temp_adapter
        .is_file()
    {
        return Err(
            "company_model_temp_adapter_missing"
                .into()
        );
    }

    let root =
        company_model_root_v1()
            .join(
                &deployment
                    .company_model_id
            )
            .join(
                format!(
                    "v{}",
                    deployment.version
                )
            );

    fs::create_dir_all(
        &root
    )
    .map_err(|_| {
        "company_model_adapter_directory_failed"
            .to_string()
    })?;

    let target =
        root.join(
            "adapter.gguf"
        );

    let staged =
        root.join(
            "adapter.gguf.tmp"
        );

    fs::copy(
        verified_temp_adapter,
        &staged,
    )
    .map_err(|_| {
        "company_model_adapter_stage_failed"
            .to_string()
    })?;

    fs::rename(
        &staged,
        &target,
    )
    .map_err(|_| {
        "company_model_adapter_commit_failed"
            .to_string()
    })?;

    let state =
        CompanyModelLocalStateV1 {
            schema_version: 1,
            company_model_id:
                deployment
                    .company_model_id
                    .clone(),
            version:
                deployment.version,
            base_model_id:
                deployment
                    .base_model_id
                    .clone(),
            base_capability:
                deployment
                    .base_capability
                    .clone(),
            adapter_sha256:
                deployment
                    .adapter_sha256
                    .to_ascii_lowercase(),
            adapter_path:
                target
                    .to_string_lossy()
                    .to_string(),
            status:
                "ready".into(),
            updated_at_unix_ms:
                now_unix_ms_v1(),
        };

    persist_local_state_v1(
        &state
    )?;

    Ok(state)
}

pub fn runtime_key_for_contract_v1(
    state: &NodeState,
    contract:
        &CompanyModelExecutionContractV1,
    selected_model:
        Option<&str>,
) -> Result<
    CompanyModelRuntimeKeyV1,
    String
> {
    if !contract
        .contract_version
        .eq_ignore_ascii_case(
            COMPANY_MODEL_EXECUTION_CONTRACT_V1
        )
    {
        return Err(
            "company_model_contract_version_invalid"
                .into()
        );
    }

    if contract
        .company_model_id
        .trim()
        .is_empty() ||
        contract.company_model_version == 0 ||
        contract
            .base_model_id
            .trim()
            .is_empty() ||
        !valid_sha256_v1(
            &contract.adapter_sha256
        )
    {
        return Err(
            "company_model_contract_invalid"
                .into()
        );
    }

    if let Some(selected) =
        selected_model
    {
        if !selected.trim().is_empty() &&
            selected != "tier:auto" &&
            !selected
                .eq_ignore_ascii_case(
                    &contract
                        .base_model_id
                )
        {
            return Err(
                "company_model_selected_base_mismatch"
                    .into()
            );
        }
    }

    if !certified_base_present_v1(
        state,
        &contract.base_model_id
    ) {
        return Err(
            "company_model_base_not_certified"
                .into()
        );
    }

    let local =
        load_local_company_model_v1()
            .ok_or_else(|| {
                "company_model_not_installed"
                    .to_string()
            })?;

    if !local
        .company_model_id
        .eq_ignore_ascii_case(
            &contract
                .company_model_id
        ) ||
        local.version !=
            contract
                .company_model_version ||
        !local
            .base_model_id
            .eq_ignore_ascii_case(
                &contract
                    .base_model_id
            ) ||
        !local
            .adapter_sha256
            .eq_ignore_ascii_case(
                &contract
                    .adapter_sha256
            )
    {
        return Err(
            "company_model_local_version_mismatch"
                .into()
        );
    }

    let adapter_path =
        PathBuf::from(
            &local.adapter_path
        );

    if !adapter_path.is_file() {
        return Err(
            "company_model_adapter_missing"
                .into()
        );
    }

    let actual_hash =
        crate::core::
            lora_training::
            sha256_file_v1(
                &adapter_path
            )?;

    if !actual_hash
        .eq_ignore_ascii_case(
            &contract
                .adapter_sha256
        )
    {
        return Err(
            "company_model_adapter_hash_mismatch"
                .into()
        );
    }

    Ok(
        CompanyModelRuntimeKeyV1 {
            company_model_id:
                contract
                    .company_model_id
                    .clone(),
            version:
                contract
                    .company_model_version,
            base_model_id:
                contract
                    .base_model_id
                    .clone(),
            adapter_sha256:
                contract
                    .adapter_sha256
                    .to_ascii_lowercase(),
            adapter_path,
        }
    )
}
