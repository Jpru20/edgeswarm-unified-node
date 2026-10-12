//! Hive-only training runtime artifact provisioning (Windows).
//! Never promotes a downloaded candidate to a certified training runtime.
//! No source in this module changes inference or joins a Hive.
use crate::core::{
    auth_client::SupabaseAuthClient,
    capacity::CapacityStatus,
    production_task_http::{refresh_auth, LocalAuth},
    NodeState,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::{blocking::Client, redirect::Policy, StatusCode, Url};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const KEY_ID: &str = "swarm-training-artifacts-ed25519-v1";
const TRAINER_NAME: &str = "edgeswarm-lora-trainer.exe";
const PYTHON_NAME: &str = "python-environment.zip";
const ENGINE: &str = "edgeswarm-qlora-windows-v1";
const API_HOST: &str = "api.edgeswarm.io";
const API_PATH: &str = "/enterprise/hive/training/runtime-offer";
const MAX_TRAINER_SIZE: u64 = 200 * 1024 * 1024;
const MAX_ENV_SIZE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_OFFER_BYTES: u64 = 64 * 1024;
const MAX_SIGNED_PAYLOAD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactFileV1 {
    pub filename: String,
    pub download_url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrainingRuntimePayloadV1 {
    pub schema_version: u8,
    pub artifact_type: String,
    pub platform: String,
    pub engine: String,
    pub gpu_backend: String,
    pub version: String,
    pub status: String,
    pub trainer: ArtifactFileV1,
    #[serde(default)]
    pub python_environment: Option<ArtifactFileV1>,
    pub supported_base_models: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedTrainingRuntimeV1 {
    pub schema_version: u8,
    pub signing_key_id: String,
    pub payload_base64: String,
    pub signature_base64: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizedOfferV1 {
    ok: bool,
    authorized: bool,
    hardware_id: String,
    artifact: Option<SignedTrainingRuntimeV1>,
}

pub fn automatic_provisioning_enabled_v1() -> bool {
    cfg!(target_os = "windows")
        && env::var("EDGESWARM_TRAINING_RUNTIME_AUTO_PROVISION_V1")
            .map(|v| v.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false)
}

pub fn published_key_present_v1() -> bool {
    option_env!("EDGESWARM_TRAINING_ARTIFACT_PUBLIC_KEY_ED25519_HEX")
        .and_then(|v| decode_hex_v1(v).ok())
        .map(|bytes| bytes.len() == 32)
        .unwrap_or(false)
}

pub fn locally_eligible_for_offer_v1(state: &NodeState) -> bool {
    cfg!(target_os = "windows")
        && state.acceleration.available
        && state.acceleration.backend.eq_ignore_ascii_case("cuda")
        && state.acceleration.vram_bytes.unwrap_or(0) >= 8 * 1024 * 1024 * 1024
        && state.models.iter().any(|model| {
            model.status == "ready"
                && model.capacity_status == CapacityStatus::Certified
                && model.certified_concurrency.unwrap_or(0) > 0
                && matches!(
                    model.selected_model.to_ascii_lowercase().as_str(),
                    "qwen2.5:3b" | "qwen2.5:7b" | "qwen2.5:14b"
                )
        })
}

fn decode_hex_v1(value: &str) -> Result<Vec<u8>, String> {
    let s = value.trim();
    if s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("training_runtime_invalid_hex".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16)
            .map_err(|_| "training_runtime_invalid_hex".to_string()))
        .collect()
}

fn valid_hash_v1(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

fn validate_download_url_v1(value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|_| "training_runtime_url_invalid".to_string())?;
    if url.scheme() != "https"
        || url.host_str() != Some(API_HOST)
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/artifacts/training-runtime/")
    {
        return Err("training_runtime_url_not_trusted".into());
    }
    Ok(())
}

fn check_file_v1(file: &ArtifactFileV1, expected: &str, max_bytes: u64) -> Result<(), String> {
    if file.filename != expected
        || !valid_hash_v1(&file.sha256)
        || file.size_bytes == 0
        || file.size_bytes > max_bytes
    {
        return Err("training_runtime_file_contract_invalid".into());
    }
    validate_download_url_v1(&file.download_url)
}

fn validate_payload_v1(p: &TrainingRuntimePayloadV1) -> Result<(), String> {
    if p.schema_version != 1
        || p.artifact_type != "training-runtime"
        || p.platform != "windows-x64"
        || p.engine != ENGINE
        || p.gpu_backend != "cuda"
        || p.version.is_empty()
        || p.version.len() > 40
        || !p.version.bytes().all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c))
        || !matches!(p.status.as_str(), "candidate" | "certified")
        || p.supported_base_models.is_empty()
        || p.supported_base_models.len() > 3
        || p.supported_base_models.iter().any(|m| {
            !matches!(m.as_str(), "qwen2.5:3b" | "qwen2.5:7b" | "qwen2.5:14b")
        })
    {
        return Err("training_runtime_payload_invalid".into());
    }
    check_file_v1(&p.trainer, TRAINER_NAME, MAX_TRAINER_SIZE)?;
    if let Some(env) = p.python_environment.as_ref() {
        check_file_v1(env, PYTHON_NAME, MAX_ENV_SIZE)?;
    }
    Ok(())
}

fn verify_signed_offer_with_key_v1(
    signed: &SignedTrainingRuntimeV1,
    trusted_key: &[u8],
) -> Result<TrainingRuntimePayloadV1, String> {
    if signed.schema_version != 1 || signed.signing_key_id != KEY_ID || trusted_key.len() != 32 {
        return Err("training_runtime_signer_not_trusted".into());
    }
    if signed.payload_base64.len() > MAX_SIGNED_PAYLOAD_BYTES * 2 {
        return Err("training_runtime_payload_too_large".into());
    }
    let payload = STANDARD.decode(&signed.payload_base64)
        .map_err(|_| "training_runtime_payload_encoding_invalid".to_string())?;
    if payload.len() > MAX_SIGNED_PAYLOAD_BYTES {
        return Err("training_runtime_payload_too_large".into());
    }
    let signature = STANDARD.decode(&signed.signature_base64)
        .map_err(|_| "training_runtime_signature_encoding_invalid".to_string())?;
    if signature.len() != 64 {
        return Err("training_runtime_signature_invalid".into());
    }
    UnparsedPublicKey::new(&ED25519, trusted_key)
        .verify(&payload, &signature)
        .map_err(|_| "training_runtime_signature_invalid".to_string())?;
    let decoded: TrainingRuntimePayloadV1 = serde_json::from_slice(&payload)
        .map_err(|_| "training_runtime_signed_payload_invalid".to_string())?;
    validate_payload_v1(&decoded)?;
    Ok(decoded)
}

fn verify_signed_offer_v1(signed: &SignedTrainingRuntimeV1)
    -> Result<TrainingRuntimePayloadV1, String>
{
    let key = option_env!("EDGESWARM_TRAINING_ARTIFACT_PUBLIC_KEY_ED25519_HEX")
        .ok_or_else(|| "training_runtime_publisher_key_not_pinned".to_string())?;
    let bytes = decode_hex_v1(key)?;
    verify_signed_offer_with_key_v1(signed, &bytes)
}

fn api_root_v1() -> Result<String, String> {
    let base = env::var("GCP_BASE_URL")
        .unwrap_or_else(|_| "https://api.edgeswarm.io".to_owned());
    let url = Url::parse(&base).map_err(|_| "training_runtime_api_url_invalid".to_string())?;
    if url.scheme() != "https"
        || url.host_str() != Some(API_HOST)
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("training_runtime_api_not_trusted".into());
    }
    Ok(format!("https://{API_HOST}"))
}

fn offer_client_v1() -> Result<Client, String> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| "training_runtime_http_client_failed".to_string())
}

fn read_bounded_json_v1<T: for<'de> Deserialize<'de>>(
    response: reqwest::blocking::Response,
) -> Result<T, String> {
    let mut bytes = Vec::new();
    response.take(MAX_OFFER_BYTES + 1).read_to_end(&mut bytes)
        .map_err(|_| "training_runtime_offer_read_failed".to_string())?;
    if bytes.len() as u64 > MAX_OFFER_BYTES {
        return Err("training_runtime_offer_too_large".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "training_runtime_offer_json_invalid".to_string())
}

fn fetch_authorized_offer_v1(
    client: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    hardware_id: &str,
) -> Result<Option<SignedTrainingRuntimeV1>, String> {
    let mut url = Url::parse(&format!("{}{}", api_root_v1()?, API_PATH))
        .map_err(|_| "training_runtime_offer_url_failed".to_string())?;
    url.query_pairs_mut().append_pair("hardwareId", hardware_id);
    for _ in 0..2 {
        let response = client.get(url.clone())
            .bearer_auth(&auth.access_token)
            .send()
            .map_err(|_| "training_runtime_offer_request_failed".to_string())?;
        if response.status() == StatusCode::UNAUTHORIZED {
            let next_auth = refresh_auth(auth_client, &auth.access_token)?;
            *auth = next_auth;
            continue;
        }
        if response.status() == StatusCode::NO_CONTENT
            || response.status() == StatusCode::NOT_FOUND
            || response.status() == StatusCode::FORBIDDEN
        {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(format!("training_runtime_offer_http_{}", response.status().as_u16()));
        }
        let offer: AuthorizedOfferV1 = read_bounded_json_v1(response)?;
        if !offer.ok {
            return Err("training_runtime_offer_not_ok".into());
        }
        if !offer.authorized {
            return Ok(None);
        }
        if !offer.hardware_id.eq_ignore_ascii_case(hardware_id) {
            return Err("training_runtime_hardware_binding_mismatch".into());
        }
        return offer.artifact.map(Some)
            .ok_or_else(|| "training_runtime_authorized_offer_missing".to_string());
    }
    Err("training_runtime_offer_auth_failed".into())
}

fn assert_safe_directory_v1(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|_| "training_runtime_directory_stat_failed".to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("training_runtime_unsafe_directory".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("training_runtime_reparse_directory_rejected".into());
        }
    }
    Ok(())
}

struct InstallLockV1 { path: PathBuf, handle: Option<File> }
impl Drop for InstallLockV1 {
    fn drop(&mut self) {
        self.handle.take();
        let _ = fs::remove_file(&self.path);
    }
}

struct StageGuardV1(PathBuf);
impl Drop for StageGuardV1 {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn hash_file_v1(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|_| "training_runtime_hash_open_failed".to_string())?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buf)
            .map_err(|_| "training_runtime_hash_read_failed".to_string())?;
        if n == 0 { break; }
        digest.update(&buf[..n]);
    }
    Ok(digest.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn verify_installed_file_v1(path: &Path, spec: &ArtifactFileV1) -> Result<(), String> {
    if !path.is_file() || fs::metadata(path).map(|m| m.len()).unwrap_or(0) != spec.size_bytes {
        return Err("training_runtime_installed_size_mismatch".into());
    }
    let actual = hash_file_v1(path)?;
    if !actual.eq_ignore_ascii_case(&spec.sha256) {
        return Err("training_runtime_installed_sha_mismatch".into());
    }
    Ok(())
}

fn download_to_new_file_v1(
    client: &Client,
    root: &Path,
    spec: &ArtifactFileV1,
) -> Result<(), String> {
    let mut response = client.get(&spec.download_url)
        .timeout(Duration::from_secs(7200))
        .send()
        .map_err(|_| "training_runtime_download_failed".to_string())?;
    if response.status() != StatusCode::OK {
        return Err(format!("training_runtime_download_http_{}", response.status().as_u16()));
    }
    if response.content_length().is_some_and(|length| length != spec.size_bytes) {
        return Err("training_runtime_download_length_mismatch".into());
    }
    let path = root.join(&spec.filename);
    let mut out = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|_| "training_runtime_download_file_create_failed".to_string())?;
    let mut bytes = 0u64;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 128 * 1024];
    loop {
        let n = response.read(&mut buf)
            .map_err(|_| "training_runtime_download_read_failed".to_string())?;
        if n == 0 { break; }
        bytes = bytes.checked_add(n as u64)
            .ok_or_else(|| "training_runtime_download_overflow".to_string())?;
        if bytes > spec.size_bytes {
            return Err("training_runtime_download_too_large".into());
        }
        out.write_all(&buf[..n])
            .map_err(|_| "training_runtime_download_write_failed".to_string())?;
        digest.update(&buf[..n]);
    }
    out.sync_all().map_err(|_| "training_runtime_file_sync_failed".to_string())?;
    if bytes != spec.size_bytes {
        return Err("training_runtime_download_incomplete".into());
    }
    let actual: String = digest.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if !actual.eq_ignore_ascii_case(&spec.sha256) {
        return Err("training_runtime_download_sha_mismatch".into());
    }
    Ok(())
}

fn stage_candidate_v1(
    client: &Client,
    signed: &SignedTrainingRuntimeV1,
    payload: &TrainingRuntimePayloadV1,
) -> Result<String, String> {
    // No `current` pointer and no `certified` manifest is written here.
    // Training eligibility always remains OFF pending separate CUDA certification.
    let app_data = crate::adapters::app_data_dir();
    assert_safe_directory_v1(&app_data)?;
    let training = app_data.join("training");
    let root = training.join("runtime");
    for dir in [&training, &root] {
        fs::create_dir_all(dir)
            .map_err(|_| "training_runtime_directory_create_failed".to_string())?;
        assert_safe_directory_v1(dir)?;
    }
    let versions = root.join("versions");
    fs::create_dir_all(&versions)
        .map_err(|_| "training_runtime_versions_directory_failed".to_string())?;
    assert_safe_directory_v1(&versions)?;

    let lock_path = root.join(".provision.lock");
    let lock_file = OpenOptions::new().create_new(true).write(true)
        .open(&lock_path)
        .map_err(|_| "training_runtime_provision_lock_busy".to_string())?;
    let _lock = InstallLockV1 { path: lock_path, handle: Some(lock_file) };

    let hash = payload.trainer.sha256.to_ascii_lowercase();
    let final_dir = versions.join(&hash);
    if final_dir.exists() {
        assert_safe_directory_v1(&final_dir)?;
        verify_installed_file_v1(&final_dir.join(TRAINER_NAME), &payload.trainer)?;
        if let Some(spec) = &payload.python_environment {
            verify_installed_file_v1(&final_dir.join(PYTHON_NAME), spec)?;
        }
        return Ok(hash);
    }

    let staged = versions.join(format!(".candidate-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&staged)
        .map_err(|_| "training_runtime_stage_directory_failed".to_string())?;
    let _stage = StageGuardV1(staged.clone());
    download_to_new_file_v1(client, &staged, &payload.trainer)?;
    if let Some(spec) = &payload.python_environment {
        download_to_new_file_v1(client, &staged, spec)?;
    }
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "engine": ENGINE,
        "status": "candidate",
        "trainerSha256": hash,
        "supportedBaseModels": payload.supported_base_models,
    });
    fs::write(staged.join("trainer-manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|_| "training_runtime_manifest_encode_failed".to_string())?)
        .map_err(|_| "training_runtime_manifest_write_failed".to_string())?;
    fs::write(staged.join("signed-offer.json"),
        serde_json::to_vec_pretty(signed).map_err(|_| "training_runtime_offer_encode_failed".to_string())?)
        .map_err(|_| "training_runtime_offer_write_failed".to_string())?;
    fs::rename(&staged, &final_dir)
        .map_err(|_| "training_runtime_atomic_stage_failed".to_string())?;
    Ok(hash)
}

/// Automatically downloads only an authorized, signed, inactive candidate.
/// It cannot enable training, set a trainer allowlist, or modify model inference.
pub fn poll_and_stage_candidate_v1(
    state: &NodeState,
    hardware_id: &str,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
) -> Result<Option<String>, String> {
    if !automatic_provisioning_enabled_v1()
        || !published_key_present_v1()
        || !locally_eligible_for_offer_v1(state)
        || !crate::core::company_model_http::company_model_node_enabled_v1()
    {
        return Ok(None);
    }
    let client = offer_client_v1()?;
    let signed = match fetch_authorized_offer_v1(&client, auth_client, auth, hardware_id)? {
        Some(value) => value,
        None => return Ok(None),
    };
    let payload = verify_signed_offer_v1(&signed)?;
    stage_candidate_v1(&client, &signed, &payload).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn payload() -> TrainingRuntimePayloadV1 {
        TrainingRuntimePayloadV1 {
            schema_version: 1,
            artifact_type: "training-runtime".into(),
            platform: "windows-x64".into(),
            engine: ENGINE.into(),
            gpu_backend: "cuda".into(),
            version: "0.1.0".into(),
            status: "candidate".into(),
            trainer: ArtifactFileV1 {
                filename: TRAINER_NAME.into(),
                download_url: "https://api.edgeswarm.io/artifacts/training-runtime/windows-x64/0.1.0/edgeswarm-lora-trainer.exe".into(),
                sha256: "a".repeat(64),
                size_bytes: 12345,
            },
            python_environment: None,
            supported_base_models: vec!["qwen2.5:3b".into()],
        }
    }

    #[test]
    fn signed_offer_authenticates_all_artifact_fields() {
        let seed = [7u8; 32];
        let pair = Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
        let bytes = serde_json::to_vec(&payload()).unwrap();
        let signed = SignedTrainingRuntimeV1 {
            schema_version: 1,
            signing_key_id: KEY_ID.into(),
            payload_base64: STANDARD.encode(&bytes),
            signature_base64: STANDARD.encode(pair.sign(&bytes).as_ref()),
        };
        assert!(verify_signed_offer_with_key_v1(&signed, pair.public_key().as_ref()).is_ok());
        let mut tampered = signed.clone();
        tampered.payload_base64 = STANDARD.encode(b"{}");
        assert!(verify_signed_offer_with_key_v1(&tampered, pair.public_key().as_ref()).is_err());
    }

    #[test]
    fn rejects_unsigned_or_untrusted_download_urls() {
        let mut p = payload();
        assert!(validate_payload_v1(&p).is_ok());
        p.trainer.download_url = "https://evil.example/installer.exe".into();
        assert!(validate_payload_v1(&p).is_err());
        p.trainer.download_url = "http://api.edgeswarm.io/artifacts/training-runtime/foo".into();
        assert!(validate_payload_v1(&p).is_err());
    }

    #[test]
    fn rejects_bad_hash_or_unknown_artifact_type() {
        let mut p = payload();
        p.trainer.sha256 = "not-a-hash".into();
        assert!(validate_payload_v1(&p).is_err());
        p.trainer.sha256 = "a".repeat(64);
        p.artifact_type = "model".into();
        assert!(validate_payload_v1(&p).is_err());
    }
}
