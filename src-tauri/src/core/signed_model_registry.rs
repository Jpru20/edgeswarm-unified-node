use crate::{
    adapters,
    core::{
        backend_client::DEFAULT_BACKEND_URL,
        result_signing::
            recover_eip191_signer_v1,
    },
};
use base64::{
    engine::general_purpose::STANDARD,
    Engine as _,
};
use reqwest::blocking::Client;
use serde::Deserialize;
use std::{
    collections::HashSet,
    env,
    fs,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
    time::{
        Duration,
        SystemTime,
        UNIX_EPOCH,
    },
};

pub const SIGNED_MODEL_REGISTRY_SCHEMA_V1:
    &str =
    "swarm-signed-model-registry-envelope-v1";

pub const MODEL_REGISTRY_PAYLOAD_VERSION_V1:
    &str =
    "swarm-model-registry-v1";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedModelArtifactV1 {
    pub artifact_role: String,
    pub filename: String,
    pub download_url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedModelDefinitionV1 {
    pub selected_model: String,
    pub artifact_id: String,
    pub capability: String,

    // SWARM_SIGNED_VISION_CAPABILITY_V1
    #[serde(default)]
    pub vision_capability: Option<String>,

    pub tier: u8,
    pub runtime: String,
    pub min_ram_gb: u16,
    pub default_ctx: u32,
    pub default_max_tokens: u32,
    pub allowed_max_tokens: u32,
    pub experimental: bool,
    pub priority: u16,
    pub quantization: String,
    pub patterns: Vec<String>,
    pub artifacts: Vec<SignedModelArtifactV1>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedModelRegistryPayloadV1 {
    pub registry_version: String,
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub models: Vec<SignedModelDefinitionV1>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignedModelRegistryEnvelopeV1 {
    schema: String,
    payload_base64: String,
    signature: String,
}

#[derive(Debug, Clone)]
pub struct VerifiedSignedModelRegistryV1 {
    pub payload: SignedModelRegistryPayloadV1,
    pub signer_address: String,
}


static ACTIVE_MODEL_REGISTRY_V171:
    OnceLock<
        RwLock<
            Option<
                VerifiedSignedModelRegistryV1
            >
        >
    > =
    OnceLock::new();

fn active_registry_store_v171()
    -> &'static RwLock<
        Option<
            VerifiedSignedModelRegistryV1
        >
    >
{
    ACTIVE_MODEL_REGISTRY_V171
        .get_or_init(|| {
            RwLock::new(None)
        })
}

pub fn install_verified_model_registry_v171(
    registry:
        VerifiedSignedModelRegistryV1,
) {
    if let Ok(mut slot) =
        active_registry_store_v171()
            .write()
    {
        *slot = Some(registry);
    }
}

pub fn active_or_cached_model_registry_v171()
    -> Option<
        VerifiedSignedModelRegistryV1
    >
{
    if let Ok(slot) =
        active_registry_store_v171()
            .read()
    {
        if let Some(registry) =
            slot.as_ref()
        {
            return Some(
                registry.clone()
            );
        }
    }

    let registry =
        load_cached_model_registry_v171()
            .ok()?;

    install_verified_model_registry_v171(
        registry.clone(),
    );

    Some(registry)
}

pub fn refresh_active_model_registry_v171()
    -> bool
{
    let Some(registry) =
        refresh_or_cached_model_registry_v171()
    else {
        return false;
    };

    install_verified_model_registry_v171(
        registry,
    );

    true
}

pub fn signed_model_definition_for_sha_v171(
    model_sha: &str,
) -> Option<SignedModelDefinitionV1> {
    active_or_cached_model_registry_v171()?
        .payload
        .models
        .into_iter()
        .find(|model| {
            model.artifacts.iter().any(
                |artifact| {
                    artifact.artifact_role
                        == "model"
                        && artifact.sha256
                            .eq_ignore_ascii_case(
                                model_sha,
                            )
                },
            )
        })
}

pub fn signed_model_definition_v171(
    selected_model: &str,
) -> Option<SignedModelDefinitionV1> {
    active_or_cached_model_registry_v171()?
        .payload
        .models
        .into_iter()
        .find(|model| {
            model.selected_model
                .eq_ignore_ascii_case(
                    selected_model
                )
        })
}

pub fn signed_vision_projector_v171(
    selected_model: &str,
) -> Option<SignedModelArtifactV1> {
    let model =
        signed_model_definition_v171(
            selected_model
        )?;

    model
        .vision_capability
        .as_ref()?;

    model
        .artifacts
        .into_iter()
        .find(|artifact| {
            artifact.artifact_role
                == "vision_projector"
        })
}

pub const TRUSTED_MODEL_REGISTRY_SIGNER_V171:
    &str =
    "0xB457e7746529b7c5c482a1EA9Bd9a35A3c5709d2";

pub fn configured_model_registry_signer_v171()
    -> Option<&'static str>
{
    Some(
        TRUSTED_MODEL_REGISTRY_SIGNER_V171
    )
}

fn now_unix_v171() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| {
            "model_registry_clock_invalid"
                .to_string()
        })
}

fn validate_sha256_v171(
    value: &str,
) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| {
                c.is_ascii_hexdigit()
            })
}

fn validate_registry_payload_v171(
    payload: &SignedModelRegistryPayloadV1,
) -> Result<(), String> {
    if payload.registry_version
        != MODEL_REGISTRY_PAYLOAD_VERSION_V1
    {
        return Err(
            "model_registry_payload_version_unsupported"
                .into()
        );
    }

    let now = now_unix_v171()?;

    if payload.issued_at_unix
        > now.saturating_add(300)
    {
        return Err(
            "model_registry_issued_in_future"
                .into()
        );
    }

    if payload.expires_at_unix <= now {
        return Err(
            "model_registry_expired"
                .into()
        );
    }

    if payload.models.is_empty() {
        return Err(
            "model_registry_models_empty"
                .into()
        );
    }

    let allowed_capabilities =
        HashSet::from([
            "Neural-Inference-3B",
            "Neural-Inference-4B",
            "Neural-Inference-7B",
            "Neural-Inference-8B",
            "Neural-Inference-14B",
            "Neural-Inference-24B",
            "Neural-Inference-27B",
            "Neural-Inference-30B",
            "Neural-Inference-70B-Plus",
            "Vision-Inference-27B",
        ]);

    let mut model_ids =
        HashSet::<String>::new();

    for model in &payload.models {
        if model.selected_model.trim().is_empty()
            || model.artifact_id.trim().is_empty()
        {
            return Err(
                "model_registry_identity_missing"
                    .into()
            );
        }

        if !model_ids.insert(
            model
                .selected_model
                .to_ascii_lowercase(),
        ) {
            return Err(
                "model_registry_duplicate_model"
                    .into()
            );
        }

        if !allowed_capabilities.contains(
            model.capability.as_str(),
        ) {
            return Err(format!(
                "model_registry_capability_unsupported:{}",
                model.capability
            ));
        }

        if let Some(vision_capability) =
            model.vision_capability.as_deref()
        {
            if !matches!(
                vision_capability,
                "Vision-Inference-27B"
                    | "Vision-Inference-30B"
                    | "Vision-Inference-70B-Plus"
            ) {
                return Err(format!(
                    "model_registry_vision_capability_unsupported:{}",
                    vision_capability
                ));
            }

            let projector_count =
                model
                    .artifacts
                    .iter()
                    .filter(|artifact| {
                        artifact.artifact_role
                            == "vision_projector"
                    })
                    .count();

            if projector_count != 1 {
                return Err(
                    "model_registry_vision_projector_required"
                        .into()
                );
            }
        }

        if model.runtime != "llama.cpp" {
            return Err(format!(
                "model_registry_runtime_unsupported:{}",
                model.runtime
            ));
        }

        if !(2..=6).contains(
            &model.tier
        ) {
            return Err(
                "model_registry_tier_invalid"
                    .into()
            );
        }

        if model.default_ctx == 0
            || model.default_max_tokens == 0
            || model.allowed_max_tokens
                < model.default_max_tokens
        {
            return Err(
                "model_registry_output_policy_invalid"
                    .into()
            );
        }

        if model.quantization.trim().is_empty()
            || model.quantization.len() > 32
        {
            return Err(
                "model_registry_quantization_invalid"
                    .into()
            );
        }

        if model.patterns.is_empty() {
            return Err(
                "model_registry_patterns_empty"
                    .into()
            );
        }

        let model_artifacts =
            model
                .artifacts
                .iter()
                .filter(|artifact| {
                    artifact.artifact_role
                        == "model"
                })
                .count();

        if model_artifacts != 1 {
            return Err(
                "model_registry_model_artifact_count_invalid"
                    .into()
            );
        }

        let model_artifact =
            model
                .artifacts
                .iter()
                .find(|artifact| {
                    artifact.artifact_role
                        == "model"
                })
                .ok_or_else(|| {
                    "model_registry_model_artifact_missing"
                        .to_string()
                })?;

        if model_artifact.filename.is_empty()
            || model_artifact
                .filename
                .contains('/')
            || model_artifact
                .filename
                .contains('\\')
        {
            return Err(
                "model_registry_filename_invalid"
                    .into()
            );
        }

        let expected_artifact_id =
            Path::new(
                &model_artifact.filename,
            )
            .file_stem()
            .and_then(|value| {
                value.to_str()
            })
            .ok_or_else(|| {
                "model_registry_artifact_id_invalid"
                    .to_string()
            })?;

        if model.artifact_id
            != expected_artifact_id
        {
            return Err(
                "model_registry_artifact_id_mismatch"
                    .into()
            );
        }

        let projector_artifacts =
            model
                .artifacts
                .iter()
                .filter(|artifact| {
                    artifact.artifact_role
                        == "vision_projector"
                })
                .count();

        if projector_artifacts > 1 {
            return Err(
                "model_registry_projector_artifact_count_invalid"
                    .into()
            );
        }

        for artifact in &model.artifacts {
            if artifact.artifact_role != "model"
                && artifact.artifact_role
                    != "vision_projector"
            {
                return Err(
                    "model_registry_artifact_role_invalid"
                        .into()
                );
            }

            if artifact.filename.is_empty()
                || artifact.filename.contains('/')
                || artifact.filename.contains('\\')
            {
                return Err(
                    "model_registry_filename_invalid"
                        .into()
                );
            }

            if !artifact
                .download_url
                .starts_with("https://")
            {
                return Err(
                    "model_registry_download_url_invalid"
                        .into()
                );
            }

            if !validate_sha256_v171(
                &artifact.sha256,
            ) {
                return Err(
                    "model_registry_sha256_invalid"
                        .into()
                );
            }
        }
    }

    Ok(())
}

pub fn verify_signed_model_registry_v171(
    raw_envelope: &str,
) -> Result<VerifiedSignedModelRegistryV1, String> {
    let expected_signer =
        configured_model_registry_signer_v171()
            .ok_or_else(|| {
                "model_registry_signer_not_configured"
                    .to_string()
            })?;

    let envelope =
        serde_json::from_str::<
            SignedModelRegistryEnvelopeV1
        >(raw_envelope)
        .map_err(|_| {
            "model_registry_envelope_invalid"
                .to_string()
        })?;

    if envelope.schema
        != SIGNED_MODEL_REGISTRY_SCHEMA_V1
    {
        return Err(
            "model_registry_envelope_version_unsupported"
                .into()
        );
    }

    let recovered =
        recover_eip191_signer_v1(
            &envelope.payload_base64,
            &envelope.signature,
        )?;

    if !recovered.eq_ignore_ascii_case(
        expected_signer,
    ) {
        return Err(
            "model_registry_signer_mismatch"
                .into()
        );
    }

    let payload_bytes =
        STANDARD
            .decode(
                envelope
                    .payload_base64
                    .as_bytes(),
            )
            .map_err(|_| {
                "model_registry_payload_base64_invalid"
                    .to_string()
            })?;

    let payload_text =
        String::from_utf8(
            payload_bytes,
        )
        .map_err(|_| {
            "model_registry_payload_utf8_invalid"
                .to_string()
        })?;

    let payload =
        serde_json::from_str::<
            SignedModelRegistryPayloadV1
        >(&payload_text)
        .map_err(|_| {
            "model_registry_payload_json_invalid"
                .to_string()
        })?;

    validate_registry_payload_v171(
        &payload,
    )?;

    Ok(
        VerifiedSignedModelRegistryV1 {
            payload,
            signer_address: recovered,
        },
    )
}

fn cache_path_v171() -> PathBuf {
    adapters::app_data_dir()
        .join(
            "signed_model_registry_v1.json"
        )
}

fn store_cache_v171(
    raw_envelope: &str,
) -> Result<(), String> {
    let path = cache_path_v171();

    if let Some(parent) =
        path.parent()
    {
        fs::create_dir_all(parent)
            .map_err(|_| {
                "model_registry_cache_directory_failed"
                    .to_string()
            })?;
    }

    let temporary =
        path.with_extension(
            "json.tmp"
        );

    fs::write(
        &temporary,
        raw_envelope.as_bytes(),
    )
    .map_err(|_| {
        "model_registry_cache_write_failed"
            .to_string()
    })?;

    if path.exists() {
        let _ =
            fs::remove_file(&path);
    }

    fs::rename(
        &temporary,
        &path,
    )
    .map_err(|_| {
        "model_registry_cache_promote_failed"
            .to_string()
    })
}

pub fn load_cached_model_registry_v171()
    -> Result<
        VerifiedSignedModelRegistryV1,
        String,
    >
{
    let raw =
        fs::read_to_string(
            cache_path_v171(),
        )
        .map_err(|_| {
            "model_registry_cache_unavailable"
                .to_string()
        })?;

    verify_signed_model_registry_v171(
        &raw,
    )
}

pub fn fetch_signed_model_registry_v171()
    -> Result<
        VerifiedSignedModelRegistryV1,
        String,
    >
{
    let base_url =
        env::var("GCP_BASE_URL")
            .unwrap_or_else(|_| {
                DEFAULT_BACKEND_URL
                    .to_string()
            });

    let response =
        Client::builder()
            .connect_timeout(
                Duration::from_secs(5)
            )
            .timeout(
                Duration::from_secs(15)
            )
            .build()
            .map_err(|_| {
                "model_registry_http_client_failed"
                    .to_string()
            })?
            .get(format!(
                "{}/node/model-registry",
                base_url.trim_end_matches('/')
            ))
            .send()
            .map_err(|_| {
                "model_registry_http_request_failed"
                    .to_string()
            })?;

    if !response.status().is_success() {
        return Err(format!(
            "model_registry_http_{}",
            response.status().as_u16()
        ));
    }

    let raw =
        response
            .text()
            .map_err(|_| {
                "model_registry_http_body_failed"
                    .to_string()
            })?;

    let verified =
        verify_signed_model_registry_v171(
            &raw,
        )?;

    store_cache_v171(&raw)?;

    Ok(verified)
}

pub fn refresh_or_cached_model_registry_v171()
    -> Option<VerifiedSignedModelRegistryV1>
{
    match fetch_signed_model_registry_v171() {
        Ok(registry) => {
            println!(
                "SIGNED_MODEL_REGISTRY_SOURCE=network"
            );

            Some(registry)
        }

        Err(network_error) => {
            println!(
                "SIGNED_MODEL_REGISTRY_NETWORK_ERROR={network_error}"
            );

            match load_cached_model_registry_v171() {
                Ok(registry) => {
                    println!(
                        "SIGNED_MODEL_REGISTRY_SOURCE=cache"
                    );

                    Some(registry)
                }

                Err(cache_error) => {
                    println!(
                        "SIGNED_MODEL_REGISTRY_CACHE_ERROR={cache_error}"
                    );

                    println!(
                        "SIGNED_MODEL_REGISTRY_SOURCE=builtin"
                    );

                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_payload_v171()
        -> SignedModelRegistryPayloadV1
    {
        let now =
            now_unix_v171().unwrap();

        SignedModelRegistryPayloadV1 {
            registry_version:
                MODEL_REGISTRY_PAYLOAD_VERSION_V1
                    .into(),
            issued_at_unix:
                now.saturating_sub(1),
            expires_at_unix:
                now.saturating_add(3600),
            models: vec![
                SignedModelDefinitionV1 {
                    selected_model:
                        "future-model:1".into(),
                    artifact_id:
                        "future-model".into(),
                    capability:
                        "Neural-Inference-30B".into(),
                    vision_capability:
                        None,
                    tier: 5,
                    runtime:
                        "llama.cpp".into(),
                    min_ram_gb: 64,
                    default_ctx: 4096,
                    default_max_tokens: 512,
                    allowed_max_tokens: 2048,
                    experimental: false,
                    priority: 10,
                    quantization:
                        "Q4_K_M".into(),
                    patterns: vec![
                        "*future-model*.gguf"
                            .into(),
                    ],
                    artifacts: vec![
                        SignedModelArtifactV1 {
                            artifact_role:
                                "model".into(),
                            filename:
                                "future-model.gguf"
                                    .into(),
                            download_url:
                                "https://api.edgeswarm.io/models/future-model.gguf"
                                    .into(),
                            sha256:
                                "a".repeat(64),
                        },
                    ],
                },
            ],
        }
    }

    #[test]
    fn production_registry_signer_is_pinned_v171() {
        assert_eq!(
            configured_model_registry_signer_v171(),
            Some(
                "0xB457e7746529b7c5c482a1EA9Bd9a35A3c5709d2"
            )
        );
    }

    #[test]
    fn valid_future_model_payload_is_accepted_v171() {
        validate_registry_payload_v171(
            &valid_payload_v171(),
        )
        .unwrap();
    }

    #[test]
    fn unsupported_runtime_is_rejected_v171() {
        let mut payload =
            valid_payload_v171();

        payload.models[0].runtime =
            "unknown-runtime".into();

        assert_eq!(
            validate_registry_payload_v171(
                &payload,
            )
            .unwrap_err(),
            "model_registry_runtime_unsupported:unknown-runtime"
        );
    }

    #[test]
    fn unsafe_artifact_filename_is_rejected_v171() {
        let mut payload =
            valid_payload_v171();

        payload.models[0]
            .artifacts[0]
            .filename =
            "../bad.gguf".into();

        assert_eq!(
            validate_registry_payload_v171(
                &payload,
            )
            .unwrap_err(),
            "model_registry_filename_invalid"
        );
    }
}
