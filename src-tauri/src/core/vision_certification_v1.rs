use crate::core::{
    adapters,
    capacity::CapacityStatus,
    model::ModelState,
    production_inference::ProductionLlamaClient,
    vision_asset_fetch_v1::PreparedVisionAssetV1,
    NodeState,
};
use crate::runtime::llama_process::{
    resolve_cpu_llama_server_path_v1,
    resolve_model_root_v1,
    runtime_acceleration_for_config_v1,
    LlamaProcessConfig,
    ManagedLlamaProcess,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

pub const VISION_CERTIFICATE_VERSION_V171: &str =
    "swarm-vision-certificate-v1";

pub const VISION_CERTIFICATION_POLICY_V171: &str =
    "swarm-vision-self-test-v1";

const CERTIFICATION_PROMPT_V171: &str =
    "Look at the image. What color is the circle? \
Reply with one lowercase English color word and nothing else.";

const EXPECTED_ANSWER_V171: &str = "blue";

const CERT_IMAGE_BYTES_V171: &[u8] =
    include_bytes!(
        "../../resources/vision-certification-v1.png"
    );

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionCertificateV171 {
    pub certificate_version: String,
    pub policy_version: String,
    pub installation_id: String,
    pub selected_model: String,
    pub vision_capability: String,
    pub model_sha256: String,
    pub projector_sha256: String,
    pub runtime_sha256: String,
    pub acceleration: String,
    pub app_version: String,
    pub test_image_sha256: String,
    pub expected_answer: String,
    pub observed_answer: String,
    pub created_at_unix_ms: u128,
}

#[derive(Debug, Clone, Default)]
pub struct VisionAutoCertificationSummaryV171 {
    pub attempted: usize,
    pub passed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone)]
struct CachedShaV171 {
    file_len: u64,
    modified_nanos: u128,
    sha256: String,
}

fn sha_cache_v171(
) -> &'static Mutex<BTreeMap<String, CachedShaV171>> {
    static CACHE:
        OnceLock<
            Mutex<
                BTreeMap<
                    String,
                    CachedShaV171
                >
            >
        > =
        OnceLock::new();

    CACHE.get_or_init(|| {
        Mutex::new(
            BTreeMap::new()
        )
    })
}

fn modified_nanos_v171(
    metadata: &fs::Metadata,
) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|value| {
            value
                .duration_since(
                    UNIX_EPOCH
                )
                .ok()
        })
        .map(|value| value.as_nanos())
        .unwrap_or(0)
}

fn sha256_cached_v171(
    path: &Path,
) -> Result<String, String> {
    let metadata =
        fs::metadata(path)
            .map_err(|_| {
                "vision_certification_file_metadata_failed"
                    .to_string()
            })?;

    if !metadata.is_file() {
        return Err(
            "vision_certification_file_not_regular"
                .into()
        );
    }

    let key =
        path.to_string_lossy()
            .to_string();

    let modified_nanos =
        modified_nanos_v171(
            &metadata
        );

    if let Ok(cache) =
        sha_cache_v171().lock()
    {
        if let Some(entry) =
            cache.get(&key)
        {
            if entry.file_len
                == metadata.len()
                && entry.modified_nanos
                    == modified_nanos
            {
                return Ok(
                    entry.sha256.clone()
                );
            }
        }
    }

    let sha256 =
        crate::core::lora_training::
            sha256_file_v1(path)?;

    if let Ok(mut cache) =
        sha_cache_v171().lock()
    {
        cache.insert(
            key,
            CachedShaV171 {
                file_len:
                    metadata.len(),
                modified_nanos,
                sha256:
                    sha256.clone(),
            },
        );
    }

    Ok(sha256)
}

fn digest_bytes_v171(
    bytes: &[u8],
) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| {
            format!("{byte:02x}")
        })
        .collect()
}

fn certificate_path_v171(
    selected_model: &str,
) -> PathBuf {
    let key =
        digest_bytes_v171(
            selected_model.as_bytes()
        );

    adapters::app_data_dir()
        .join(
            "vision-certificates-v1"
        )
        .join(
            format!("{key}.json")
        )
}

fn write_certificate_v171(
    certificate: &VisionCertificateV171,
) -> Result<PathBuf, String> {
    let path =
        certificate_path_v171(
            &certificate.selected_model
        );

    let parent =
        path.parent()
            .ok_or_else(|| {
                "vision_certificate_parent_missing"
                    .to_string()
            })?;

    fs::create_dir_all(parent)
        .map_err(|_| {
            "vision_certificate_directory_failed"
                .to_string()
        })?;

    let encoded =
        serde_json::to_vec_pretty(
            certificate
        )
        .map_err(|_| {
            "vision_certificate_encode_failed"
                .to_string()
        })?;

    let temporary =
        path.with_extension(
            "json.download"
        );

    fs::write(
        &temporary,
        encoded
    )
    .map_err(|_| {
        "vision_certificate_write_failed"
            .to_string()
    })?;

    if path.exists() {
        let _ =
            fs::remove_file(
                &path
            );
    }

    fs::rename(
        &temporary,
        &path
    )
    .map_err(|_| {
        "vision_certificate_commit_failed"
            .to_string()
    })?;

    Ok(path)
}

fn read_certificate_v171(
    selected_model: &str,
) -> Option<VisionCertificateV171> {
    let path =
        certificate_path_v171(
            selected_model
        );

    let bytes =
        fs::read(path)
            .ok()?;

    serde_json::from_slice(
        &bytes
    )
    .ok()
}

fn signed_artifacts_v171(
    selected_model: &str,
) -> Option<(
    crate::core::signed_model_registry::
        SignedModelDefinitionV1,
    crate::core::signed_model_registry::
        SignedModelArtifactV1,
    crate::core::signed_model_registry::
        SignedModelArtifactV1,
)> {
    let definition =
        crate::core::signed_model_registry::
            signed_model_definition_v171(
                selected_model
            )?;

    let model =
        definition
            .artifacts
            .iter()
            .find(|artifact| {
                artifact.artifact_role
                    == "model"
            })?
            .clone();

    let projector =
        definition
            .artifacts
            .iter()
            .find(|artifact| {
                artifact.artifact_role
                    == "vision_projector"
            })?
            .clone();

    Some((
        definition,
        model,
        projector,
    ))
}

fn execution_config_v171(
    model_path: &Path,
    projector_path: &Path,
    acceleration: &str,
) -> Result<LlamaProcessConfig, String> {
    let mut config =
        LlamaProcessConfig::for_model(
            model_path
                .to_string_lossy()
                .to_string()
        )?;

    #[cfg(target_os = "windows")]
    {
        match acceleration {
            "cpu" => {
                config.executable =
                    resolve_cpu_llama_server_path_v1()?;

                config.gpu_layers = 0;
            }

            "cuda" | "vulkan" => {
                let actual =
                    runtime_acceleration_for_config_v1(
                        &config
                    );

                if actual != acceleration {
                    return Err(format!(
                        "vision_certification_runtime_acceleration_mismatch:expected={acceleration}:actual={actual}"
                    ));
                }
            }

            _ => {}
        }
    }

    config.vision_projector_path =
        Some(
            projector_path.to_path_buf()
        );

    Ok(config)
}

fn answer_passes_v171(
    value: &str,
) -> bool {
    let normalized =
        value
            .trim()
            .trim_matches(
                |c: char| {
                    matches!(
                        c,
                        '"' | '\''
                            | '.'
                            | ','
                            | '!'
                            | '?'
                            | '*'
                    )
                }
            )
            .trim()
            .to_ascii_lowercase();

    normalized
        == EXPECTED_ANSWER_V171
}

pub fn vision_certificate_valid_v171(
    state: &NodeState,
    model: &ModelState,
) -> bool {
    if model.status != "ready"
        || model.capacity_status
            != CapacityStatus::Certified
        || model
            .certified_concurrency
            .unwrap_or(0)
            == 0
    {
        return false;
    }

    let Some((
        definition,
        model_artifact,
        projector_artifact,
    )) =
        signed_artifacts_v171(
            &model.selected_model
        )
    else {
        return false;
    };

    let Some(vision_capability) =
        definition.vision_capability
    else {
        return false;
    };

    let Some(certificate) =
        read_certificate_v171(
            &model.selected_model
        )
    else {
        return false;
    };

    let root =
        match resolve_model_root_v1() {
            Ok(root) => root,
            Err(_) => return false,
        };

    let model_path =
        root.join(
            &model_artifact.filename
        );

    let projector_path =
        root.join(
            &projector_artifact.filename
        );

    if !model_path.is_file()
        || !projector_path.is_file()
    {
        return false;
    }

    let config =
        match execution_config_v171(
            &model_path,
            &projector_path,
            &model.acceleration,
        ) {
            Ok(config) => config,
            Err(_) => return false,
        };

    let runtime_sha256 =
        match sha256_cached_v171(
            &config.executable
        ) {
            Ok(value) => value,
            Err(_) => return false,
        };

    let test_image_sha256 =
        digest_bytes_v171(
            CERT_IMAGE_BYTES_V171
        );

    certificate.certificate_version
        == VISION_CERTIFICATE_VERSION_V171
        && certificate.policy_version
            == VISION_CERTIFICATION_POLICY_V171
        && certificate.installation_id
            == state.identity.installation_id
        && certificate.selected_model
            == model.selected_model
        && certificate.vision_capability
            == vision_capability
        && certificate
            .model_sha256
            .eq_ignore_ascii_case(
                &model_artifact.sha256
            )
        && certificate
            .projector_sha256
            .eq_ignore_ascii_case(
                &projector_artifact.sha256
            )
        && certificate
            .runtime_sha256
            .eq_ignore_ascii_case(
                &runtime_sha256
            )
        && certificate.acceleration
            == model.acceleration
        && certificate.app_version
            == env!("CARGO_PKG_VERSION")
        && certificate
            .test_image_sha256
            .eq_ignore_ascii_case(
                &test_image_sha256
            )
        && certificate.expected_answer
            == EXPECTED_ANSWER_V171
        && answer_passes_v171(
            &certificate.observed_answer
        )
}

fn certify_one_vision_model_v171(
    state: &NodeState,
    model: &ModelState,
) -> Result<(), String> {
    let (
        definition,
        model_artifact,
        projector_artifact,
    ) =
        signed_artifacts_v171(
            &model.selected_model
        )
        .ok_or_else(|| {
            "vision_certification_signed_definition_missing"
                .to_string()
        })?;

    let vision_capability =
        definition
            .vision_capability
            .clone()
            .ok_or_else(|| {
                "vision_certification_capability_missing"
                    .to_string()
            })?;

    let root =
        resolve_model_root_v1()?;

    let model_path =
        root.join(
            &model_artifact.filename
        );

    let projector_path =
        root.join(
            &projector_artifact.filename
        );

    if !model_path.is_file() {
        return Err(
            "vision_certification_model_missing"
                .into()
        );
    }

    if !projector_path.is_file() {
        return Err(
            "vision_certification_projector_missing"
                .into()
        );
    }

    let actual_projector_sha =
        sha256_cached_v171(
            &projector_path
        )?;

    if !actual_projector_sha
        .eq_ignore_ascii_case(
            &projector_artifact.sha256
        )
    {
        return Err(
            "vision_certification_projector_sha_mismatch"
                .into()
        );
    }

    let mut config =
        execution_config_v171(
            &model_path,
            &projector_path,
            &model.acceleration,
        )?;

    config.startup_timeout =
        std::time::Duration::from_secs(
            180
        );

    let runtime_sha256 =
        sha256_cached_v171(
            &config.executable
        )?;

    let image_root =
        adapters::app_data_dir()
            .join(
                "vision-certification-v1"
            );

    fs::create_dir_all(
        &image_root
    )
    .map_err(|_| {
        "vision_certification_image_directory_failed"
            .to_string()
    })?;

    let image_path =
        image_root.join(
            "vision-certification-v1.png"
        );

    fs::write(
        &image_path,
        CERT_IMAGE_BYTES_V171
    )
    .map_err(|_| {
        "vision_certification_image_write_failed"
            .to_string()
    })?;

    let test_image_sha256 =
        digest_bytes_v171(
            CERT_IMAGE_BYTES_V171
        );

    let asset =
        PreparedVisionAssetV1 {
            asset_id:
                "swarm_vision_cert_v1"
                    .into(),
            media_type:
                "image/png".into(),
            sha256:
                test_image_sha256.clone(),
            size_bytes:
                CERT_IMAGE_BYTES_V171
                    .len() as u64,
            path:
                image_path,
        };

    println!(
        "AUTOMATIC_VISION_CERTIFICATION_RUNTIME_START model={}",
        model.selected_model
    );

    let runtime =
        ManagedLlamaProcess::start(
            &config
        )?;

    let client =
        ProductionLlamaClient::new(
            runtime
                .base_url()
                .to_string()
        )?;

    client.health_check()?;

    let result =
        client.execute_vision(
            CERTIFICATION_PROMPT_V171,
            Some(8),
            &[asset],
        )?;

    let parsed: Value =
        serde_json::from_str(
            &result.ai_output
        )
        .map_err(|_| {
            "vision_certification_output_invalid"
                .to_string()
        })?;

    let observed =
        parsed
            .get("response")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

    if !answer_passes_v171(
        &observed
    ) {
        return Err(format!(
            "vision_certification_answer_mismatch:observed={observed}"
        ));
    }

    let created_at_unix_ms =
        SystemTime::now()
            .duration_since(
                UNIX_EPOCH
            )
            .map_err(|_| {
                "vision_certification_clock_invalid"
                    .to_string()
            })?
            .as_millis();

    let certificate =
        VisionCertificateV171 {
            certificate_version:
                VISION_CERTIFICATE_VERSION_V171
                    .into(),
            policy_version:
                VISION_CERTIFICATION_POLICY_V171
                    .into(),
            installation_id:
                state.identity
                    .installation_id
                    .clone(),
            selected_model:
                model.selected_model
                    .clone(),
            vision_capability,
            model_sha256:
                model_artifact.sha256,
            projector_sha256:
                projector_artifact.sha256,
            runtime_sha256,
            acceleration:
                model.acceleration
                    .clone(),
            app_version:
                env!("CARGO_PKG_VERSION")
                    .into(),
            test_image_sha256,
            expected_answer:
                EXPECTED_ANSWER_V171
                    .into(),
            observed_answer:
                observed,
            created_at_unix_ms,
        };

    let path =
        write_certificate_v171(
            &certificate
        )?;

    println!(
        "AUTOMATIC_VISION_CERTIFICATION_CERTIFICATE={}",
        path.display()
    );

    println!(
        "AUTOMATIC_VISION_CERTIFICATION_PASS=true"
    );

    Ok(())
}

pub fn auto_certify_pending_vision_models_v171(
    state: &NodeState,
) -> VisionAutoCertificationSummaryV171 {
    let mut summary =
        VisionAutoCertificationSummaryV171::default();

    for model in &state.models {
        if model.status != "ready"
            || model.capacity_status
                != CapacityStatus::Certified
            || model
                .certified_concurrency
                .unwrap_or(0)
                == 0
        {
            continue;
        }

        let has_vision =
            crate::core::signed_model_registry::
                signed_model_definition_v171(
                    &model.selected_model
                )
                .and_then(|definition| {
                    definition
                        .vision_capability
                })
                .is_some();

        if !has_vision {
            continue;
        }

        if vision_certificate_valid_v171(
            state,
            model,
        ) {
            println!(
                "AUTOMATIC_VISION_CERTIFICATION_REUSED=true model={}",
                model.selected_model
            );

            continue;
        }

        summary.attempted += 1;

        println!(
            "AUTOMATIC_VISION_CERTIFICATION_MODEL={}",
            model.selected_model
        );

        match certify_one_vision_model_v171(
            state,
            model,
        ) {
            Ok(()) => {
                summary.passed += 1;
            }

            Err(error) => {
                summary.failed += 1;

                println!(
                    "AUTOMATIC_VISION_CERTIFICATION_ERROR={} model={}",
                    error,
                    model.selected_model
                );
            }
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_color_answer_passes_v171() {
        assert!(
            answer_passes_v171(
                "blue"
            )
        );

        assert!(
            answer_passes_v171(
                "Blue."
            )
        );

        assert!(
            !answer_passes_v171(
                "the circle is blue"
            )
        );
    }

    #[test]
    fn certification_image_is_embedded_v171() {
        assert!(
            CERT_IMAGE_BYTES_V171
                .starts_with(
                    b"\x89PNG\r\n\x1a\n"
                )
        );

        assert!(
            CERT_IMAGE_BYTES_V171.len()
                > 100
        );
    }
}
