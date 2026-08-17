use crate::{
    adapters,
    core::{
        capacity::{
            CapacityState,
            CapacityStatus,
        },
        capacity_store::load_certificate,
        certificate_match::{
            certificate_match_failures,
            sha256_file,
            CertificateMatchContext,
        },
        model::ModelState,
        model_registry::OUTPUT_LIMIT_POLICY_VERSION,
    },
};
use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const MODEL_ID: &str = "Qwen2.5-3B-Instruct-Q4_K_M";
const MODEL_CAPABILITY: &str = "Neural-Inference-3B";
const QUANTIZATION: &str = "Q4_K_M";
const RUNTIME: &str = "llama.cpp";
const PACK_ID: &str = "edgeswarm-3b-realworld-v2";
const BENCHMARK_MODE: &str = "no_cache_prompt";
const CAPACITY_POLICY_VERSION: &str = "realworld-capacity-policy-v1";

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

pub fn detect_installed_capability(
    installation_id: &str,
    acceleration: &str,
) -> (Vec<ModelState>, CapacityState) {
    let model_path = match env::var("EDGESWARM_MODEL_PATH") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => return (Vec::new(), CapacityState::default()),
    };

    let runtime_path = match env::var("EDGESWARM_RUNTIME_PATH") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => {
            return (
                vec![ModelState {
                    selected_model: "qwen2.5:3b".into(),
                    model_id: MODEL_ID.into(),
                    capability: MODEL_CAPABILITY.into(),
                    tier: 2,
                    runtime: RUNTIME.into(),
                    acceleration: acceleration.into(),
                    status: "revalidation_required".into(),
                    capacity_status: CapacityStatus::RevalidationRequired,
                    certified_concurrency: None,
                }],
                CapacityState {
                    status: CapacityStatus::RevalidationRequired,
                    ..CapacityState::default()
                },
            );
        }
    };

    if !model_path.is_file() {
        return (Vec::new(), CapacityState::default());
    }

    let mut model = ModelState {
        selected_model: "qwen2.5:3b".into(),
        model_id: MODEL_ID.into(),
        capability: MODEL_CAPABILITY.into(),
        tier: 2,
        runtime: RUNTIME.into(),
        acceleration: acceleration.into(),
        status: "installed_uncertified".into(),
        capacity_status: CapacityStatus::Uncertified,
        certified_concurrency: None,
    };

    let actual_sha = match sha256_file(&model_path) {
        Ok(value) => value,
        Err(_) => {
            model.status = "revalidation_required".into();
            model.capacity_status = CapacityStatus::RevalidationRequired;
            model.certified_concurrency = None;

            return (
                vec![model],
                CapacityState {
                    status: CapacityStatus::RevalidationRequired,
                    ..CapacityState::default()
                },
            );
        }
    };

    let actual_runtime_version = match runtime_version(&runtime_path) {
        Ok(value) => value,
        Err(_) => {
            model.status = "revalidation_required".into();
            model.capacity_status = CapacityStatus::RevalidationRequired;
            model.certified_concurrency = None;

            return (
                vec![model],
                CapacityState {
                    status: CapacityStatus::RevalidationRequired,
                    ..CapacityState::default()
                },
            );
        }
    };

    let directory = match certificate_directory() {
        Ok(value) => value,
        Err(_) => return (vec![model], CapacityState::default()),
    };

    let entries = match fs::read_dir(directory) {
        Ok(value) => value,
        Err(_) => return (vec![model], CapacityState::default()),
    };

    let mut matching_certificate_seen = false;

    for entry in entries.flatten() {
        let path = entry.path();

        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }

        let certificate = match load_certificate(&path) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if certificate.model_id != MODEL_ID ||
            certificate.model_sha256 != actual_sha
        {
            continue;
        }

        matching_certificate_seen = true;

        let context = CertificateMatchContext {
            installation_id,
            model_id: MODEL_ID,
            model_sha256: &actual_sha,
            model_capability: MODEL_CAPABILITY,
            quantization: QUANTIZATION,
            runtime: RUNTIME,
            runtime_version: &actual_runtime_version,
            acceleration,
            certification_pack_id: PACK_ID,
            benchmark_mode: BENCHMARK_MODE,
            capacity_policy_version: CAPACITY_POLICY_VERSION,
            output_limit_policy_version: OUTPUT_LIMIT_POLICY_VERSION,
        };

        let failures =
            certificate_match_failures(&certificate, &context);

        if failures.is_empty() {
            model.status = "ready".into();
            model.capacity_status = CapacityStatus::Certified;
            model.certified_concurrency =
                Some(certificate.certified_concurrency);

            return (
                vec![model],
                CapacityState {
                    certified_concurrency:
                        certificate.certified_concurrency,
                    burst_concurrency:
                        certificate.burst_concurrency,
                    status: CapacityStatus::Certified,
                    baseline_tokens_per_second:
                        Some(certificate.baseline_tokens_per_second),
                    certified_tokens_per_second:
                        Some(certificate.certified_tokens_per_second),
                    certificates: vec![certificate],
                },
            );
        }
    }

    if matching_certificate_seen {
        model.status = "revalidation_required".into();

        (
            vec![model],
            CapacityState {
                status: CapacityStatus::RevalidationRequired,
                ..CapacityState::default()
            },
        )
    } else {
        (vec![model], CapacityState::default())
    }
}
