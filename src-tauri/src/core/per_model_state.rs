use crate::{
    adapters,
    core::{
        capacity::{CapacityCertificateV1, CapacityStatus},
        capacity_store::load_certificate,
        certificate_match::{certificate_match_failures, CertificateMatchContext},
        model::ModelState,
        model_discovery::{discover_models, DiscoveredModelV1},
        model_fingerprint::resolve_model_fingerprint,
        model_registry::OUTPUT_LIMIT_POLICY_VERSION,
    },
};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerModelStateV1 {
    pub selected_model: String,
    pub artifact_id: String,
    pub artifact_path: String,
    pub capability: String,
    pub tier: u8,
    pub runtime: String,
    pub runtime_version: String,
    pub acceleration: String,
    pub status: String,
    pub capacity_status: CapacityStatus,
    pub certified_concurrency: Option<u16>,
    pub fingerprint_resolved: bool,
    pub fingerprint_cache_hit: Option<bool>,
    pub certificate_loaded: bool,
}

struct CertificationPolicyV1 {
    pack_id: &'static str,
    capacity_policy_version: &'static str,
    quantization: &'static str,
}

fn certification_policy(capability: &str) -> Option<CertificationPolicyV1> {
    if !capability.starts_with("Neural-Inference") {
        return None;
    }

    Some(CertificationPolicyV1 {
        pack_id: crate::core::certification_workload::NEURAL_REALWORLD_PACK_ID_V1,
        capacity_policy_version: "realworld-capacity-policy-v1",
        quantization: "Q4_K_M",
    })
}

fn runtime_version(path: &Path) -> Result<String, String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|e| format!("runtime_version_failed:{e}"))?;

    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    for line in text.lines() {
        if let Some(version) = line.trim().strip_prefix("version: ") {
            return Ok(version.trim().to_string());
        }
    }

    Err("runtime_version_not_found".into())
}

fn certificate_directory() -> Result<PathBuf, String> {
    let identity_path = adapters::identity_file_path();

    let parent = identity_path
        .parent()
        .ok_or_else(|| "identity_parent_missing".to_string())?;

    Ok(parent.join("capacity_certificates"))
}

fn load_certificates() -> Vec<CapacityCertificateV1> {
    let Ok(directory) = certificate_directory() else {
        return Vec::new();
    };

    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();

            if path.extension().and_then(|v| v.to_str()) != Some("json") {
                return None;
            }

            load_certificate(&path).ok()
        })
        .collect()
}

fn artifact_id(model: &DiscoveredModelV1) -> String {
    model
        .path
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or(&model.file_name)
        .to_string()
}

pub fn resolve_per_model_states(
    model_root: &Path,
    runtime_path: &Path,
    installation_id: &str,
    acceleration: &str,
) -> Result<Vec<PerModelStateV1>, String> {
    let runtime_version = runtime_version(runtime_path)?;
    let certificates = load_certificates();

    let mut states = Vec::new();

    for model in discover_models(model_root) {
        let artifact_id = artifact_id(&model);

        let mut state = PerModelStateV1 {
            selected_model: model.selected_model.into(),
            artifact_id: artifact_id.clone(),
            artifact_path: model.path.to_string_lossy().to_string(),
            capability: model.capability.into(),
            tier: model.tier,
            runtime: model.runtime.into(),
            runtime_version: runtime_version.clone(),
            acceleration: acceleration.into(),
            status: "installed_uncertified".into(),
            capacity_status: CapacityStatus::Uncertified,
            certified_concurrency: None,
            fingerprint_resolved: false,
            fingerprint_cache_hit: None,
            certificate_loaded: false,
        };

        let Some(policy) = certification_policy(model.capability) else {
            states.push(state);
            continue;
        };

        let candidates: Vec<&CapacityCertificateV1> = certificates
            .iter()
            .filter(|certificate| {
                certificate.model_capability == model.capability
                    && certificate.runtime == model.runtime
                    && (certificate.certification_pack_id == policy.pack_id
                        || (certificate.model_capability == "Neural-Inference-3B"
                            && certificate.certification_pack_id
                                == crate::core::certification_workload::LEGACY_NEURAL_REALWORLD_PACK_ID_V1))
            })
            .collect();

        if candidates.is_empty() {
            states.push(state);
            continue;
        }

        let fingerprint = match resolve_model_fingerprint(model.selected_model, &model.path, false)
        {
            Ok(value) => value,
            Err(_) => {
                state.status = "revalidation_required".into();
                state.capacity_status = CapacityStatus::RevalidationRequired;
                states.push(state);
                continue;
            }
        };

        state.fingerprint_resolved = true;
        state.fingerprint_cache_hit = Some(fingerprint.cache_hit);

        let mut relevant_certificate_seen = false;

        for certificate in candidates {
            if certificate.model_sha256 != fingerprint.sha256 {
                continue;
            }

            relevant_certificate_seen = true;
            state.certificate_loaded = true;

            let context = CertificateMatchContext {
                installation_id,
                model_id: &artifact_id,
                model_sha256: &fingerprint.sha256,
                model_capability: model.capability,
                quantization: policy.quantization,
                runtime: model.runtime,
                runtime_version: &runtime_version,
                acceleration,
                certification_pack_id: certificate.certification_pack_id.as_str(),
                benchmark_mode: "no_cache_prompt",
                capacity_policy_version: policy.capacity_policy_version,
                output_limit_policy_version: OUTPUT_LIMIT_POLICY_VERSION,
            };

            let failures = certificate_match_failures(certificate, &context);

            if failures.is_empty() {
                state.status = "ready".into();
                state.capacity_status = CapacityStatus::Certified;
                state.certified_concurrency = Some(certificate.certified_concurrency);

                break;
            }

            state.status = "revalidation_required".into();

            state.capacity_status = CapacityStatus::RevalidationRequired;
        }

        if !relevant_certificate_seen {
            state.status = "revalidation_required".into();

            state.capacity_status = CapacityStatus::RevalidationRequired;
        }

        states.push(state);
    }

    states.sort_by(|left, right| {
        left.selected_model
            .cmp(&right.selected_model)
            .then_with(|| {
                let left_rank = if left.status == "ready" { 0 } else { 1 };
                let right_rank = if right.status == "ready" { 0 } else { 1 };

                left_rank
                    .cmp(&right_rank)
                    .then_with(|| left.artifact_path.cmp(&right.artifact_path))
            })
    });

    states.dedup_by(|left, right| left.selected_model == right.selected_model);

    Ok(states)
}

impl From<&PerModelStateV1> for ModelState {
    fn from(state: &PerModelStateV1) -> Self {
        Self {
            selected_model: state.selected_model.clone(),
            model_id: state.artifact_id.clone(),
            capability: state.capability.clone(),
            tier: state.tier,
            runtime: state.runtime.clone(),
            acceleration: state.acceleration.clone(),
            status: state.status.clone(),
            capacity_status: state.capacity_status.clone(),
            certified_concurrency: state.certified_concurrency,
        }
    }
}
