//! Explicit signed retirement, restricted to files installed by Swarm.
//! An inactive/low-priority model is not implicitly disposable.
use crate::{
    adapters,
    core::{
        capacity::CapacityStatus,
        certificate_match::sha256_file,
        model_discovery::discover_models,
        model_provisioning::ModelArtifactV1,
        signed_model_registry::{
            SignedModelDefinitionV1, SignedModelRegistryPayloadV1,
            VerifiedSignedModelRegistryV1, WindowsModelRetirementV177,
        },
        NodeState,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::Write,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const GRACE_SECONDS: u64 = 7 * 24 * 60 * 60;
const MAX_OBSERVATION_GAP: u64 = 24 * 60 * 60;
const REPARSE_POINT: u32 = 0x400;
const LEDGER: &str = "swarm-retirement-state-v177.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct OwnedArtifact {
    version: u8,
    filename: String,
    sha256: String,
    bytes: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Observation {
    rule: WindowsModelRetirementV177,
    replacement_sha256: String,
    owned: OwnedArtifact,
    first_healthy_unix: u64,
    last_healthy_unix: u64,
    last_registry_issued: u64,
}

pub(crate) struct RetirementCandidate {
    pub retired_model: String,
    pub replacement_model: String,
    root: PathBuf,
    owned: OwnedArtifact,
    replacement: OwnedArtifact,
    expires_at_unix: u64,
}

fn now_unix() -> Result<u64, String> {
    SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs()).map_err(|_| "retirement_clock_invalid".into())
}

fn safe_filename(name: &str) -> bool {
    name.len() <= 240 && !name.starts_with('.')
        && name.to_ascii_lowercase().ends_with(".gguf")
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

fn plain_file(path: &Path) -> Option<fs::Metadata> {
    let m = fs::symlink_metadata(path).ok()?;
    (m.is_file() && m.file_attributes() & REPARSE_POINT == 0).then_some(m)
}

fn plain_directory(path: &Path) -> bool {
    path.ancestors().filter(|p| !p.as_os_str().is_empty()).all(|p| {
        fs::symlink_metadata(p).map(|m|
            m.is_dir() && m.file_attributes() & REPARSE_POINT == 0
        ).unwrap_or(false)
    })
}

fn managed_root() -> Option<PathBuf> {
    // Never adopt or remove an artifact from a custom/shared model directory.
    if env::var_os("EDGESWARM_MODEL_ROOT").is_some() { return None; }
    let root = adapters::app_data_dir().join("models");
    if !plain_directory(&root) { return None; }
    let detected = crate::runtime::llama_process::resolve_model_root_v1().ok()?;
    if fs::canonicalize(&root).ok()? != fs::canonicalize(detected).ok()? { return None; }
    Some(root)
}

fn identity(path: &Path, name: &str, sha: &str) -> Option<OwnedArtifact> {
    if !safe_filename(name) || path.file_name()?.to_str()? != name { return None; }
    let m = plain_file(path)?;
    let modified = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(OwnedArtifact {
        version: 1,
        filename: name.into(),
        sha256: sha.to_ascii_lowercase(),
        bytes: m.len(),
        modified_secs: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn receipt_path(root: &Path, name: &str) -> PathBuf {
    root.join(format!("{name}.swarm-owned-v177.json"))
}

fn read_receipt(root: &Path, name: &str, sha: &str) -> Option<OwnedArtifact> {
    if !safe_filename(name) { return None; }
    let path = receipt_path(root, name);
    if plain_file(&path)?.len() > 4096 { return None; }
    let receipt: OwnedArtifact = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let actual = identity(&root.join(name), name, sha)?;
    (receipt == actual).then_some(receipt)
}

// Called only after the downloader installs a previously absent, SHA-verified
// file. Merely discovering an existing matching GGUF never establishes ownership.
pub(crate) fn record_new_download(root: &Path, artifact: &ModelArtifactV1) {
    let Some(managed) = managed_root() else { return; };
    if fs::canonicalize(root).ok() != fs::canonicalize(managed).ok()
        || artifact.artifact_role != "model" { return; }
    let Some(record) = identity(&root.join(&artifact.filename), &artifact.filename, &artifact.sha256)
        else { return; };
    let path = receipt_path(root, &record.filename);
    if let Ok(bytes) = serde_json::to_vec(&record) {
        if let Ok(mut file) = fs::OpenOptions::new().write(true).create_new(true).open(path) {
            if file.write_all(&bytes).and_then(|_| file.sync_all()).is_err() {
                println!("MODEL_RETIREMENT_OWNERSHIP_RECEIPT_FAILED=true");
            }
        }
    }
}

fn load_ledger(root: &Path) -> Result<Vec<Observation>, String> {
    let path = root.join(LEDGER);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Ok(_) if plain_file(&path).map(|m| m.len() <= 262144) == Some(true) => {
            // A malformed ledger cannot authorize an early deletion.
            Ok(serde_json::from_slice(&fs::read(path)
                .map_err(|_| "retirement_ledger_read_failed")?).unwrap_or_default())
        }
        _ => Err("retirement_ledger_path_unsafe".into()),
    }
}

fn save_ledger(root: &Path, entries: &[Observation]) -> Result<(), String> {
    let path = root.join(LEDGER);
    if entries.is_empty() && !path.exists() { return Ok(()); }
    if fs::symlink_metadata(&path).is_ok() && plain_file(&path).is_none() {
        return Err("retirement_ledger_path_unsafe".into());
    }
    // A unique create-new temporary file avoids following an existing link.
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_| "retirement_clock_invalid")?.as_nanos();
    let temporary = root.join(format!("{LEDGER}.{}.{}.tmp", std::process::id(), nonce));
    let bytes = serde_json::to_vec(entries).map_err(|_| "retirement_ledger_serialize_failed")?;
    let result = (|| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
        // Invalidate an old ledger rather than accidentally reuse its grace.
        let _ = fs::remove_file(root.join(LEDGER));
        return Err("retirement_ledger_write_failed".into());
    }
    Ok(())
}

// Record the impending activation before loading the model. Returning to an
// older model (including a claimed task or a rollback) resets its grace clock.
pub(crate) fn note_activation(active: &str) -> Result<(), String> {
    let Some(root) = managed_root() else { return Ok(()); };
    let mut entries = load_ledger(&root)?;
    let count = entries.len();
    entries.retain(|e| e.rule.replacement_model == active);
    if entries.len() != count { save_ledger(&root, &entries)?; }
    Ok(())
}

fn certified(state: &NodeState, definition: &SignedModelDefinitionV1) -> bool {
    state.models.iter().any(|m| m.selected_model == definition.selected_model
        && m.model_id == definition.artifact_id
        && m.status == "ready" && m.capacity_status == CapacityStatus::Certified
        && m.capability == definition.capability
        && m.tier == definition.tier && m.runtime == definition.runtime
        && m.certified_concurrency.unwrap_or(0) > 0)
}

fn definition<'a>(payload: &'a SignedModelRegistryPayloadV1, model: &str)
    -> Option<&'a SignedModelDefinitionV1> {
    payload.models.iter().find(|m| m.selected_model == model)
}

fn replacement_artifact(root: &Path, model: &SignedModelDefinitionV1) -> Option<OwnedArtifact> {
    let artifact = model.artifacts.iter().find(|a| a.artifact_role == "model")?;
    identity(&root.join(&artifact.filename), &artifact.filename, &artifact.sha256)
}

fn exact_discovery(root: &Path, model: &str, filename: &str) -> bool {
    let paths: Vec<_> = discover_models(root).into_iter()
        .filter(|m| m.selected_model == model).map(|m| m.path).collect();
    paths.len() == 1 && fs::canonicalize(&paths[0]).ok()
        == fs::canonicalize(root.join(filename)).ok()
}

fn protected_company_or_training() -> bool {
    // Be conservative for any company deployment, even if it is not ready.
    let company = crate::core::company_model::company_model_root_v1();
    let company_data = match fs::read_dir(company) {
        Ok(mut entries) => entries.next().is_some(),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    };
    company_data
        || crate::core::company_model_http::company_model_node_enabled_v1()
        || crate::core::training_runtime_provisioning_v1::automatic_provisioning_enabled_v1()
}

pub(crate) fn skip_retired_download(state: &NodeState, artifact: &ModelArtifactV1) -> bool {
    let Some(registry) = crate::core::signed_model_registry::active_or_cached_model_registry_v171()
        else { return false; };
    let payload = &registry.payload;
    if now_unix().map(|n| n >= payload.expires_at_unix).unwrap_or(true)
        || managed_root().is_none() || protected_company_or_training() { return false; }
    payload.windows_model_retirements_v177.iter().any(|rule| {
        artifact.model_id.as_deref() == Some(rule.retired_model.as_str())
            && artifact.artifact_role == "model"
            && artifact.sha256.eq_ignore_ascii_case(&rule.artifact_sha256)
            && definition(payload, &rule.retired_model)
                .map(|m| m.artifacts[0].filename == artifact.filename).unwrap_or(false)
            && definition(payload, &rule.replacement_model)
                .map(|m| certified(state, m)).unwrap_or(false)
    })
}

fn continuous_observation(
    old: &Observation, rule: &WindowsModelRetirementV177, owned: &OwnedArtifact,
    replacement: &OwnedArtifact, now: u64, issued: u64,
) -> bool {
    old.rule == *rule && old.owned == *owned && old.replacement_sha256 == replacement.sha256
        && old.first_healthy_unix >= rule.deprecated_at_unix
        && old.first_healthy_unix <= old.last_healthy_unix
        && now >= old.last_healthy_unix
        && now - old.last_healthy_unix <= MAX_OBSERVATION_GAP
        && issued >= old.last_registry_issued
}

fn grace_elapsed(first: u64, now: u64) -> bool {
    now.checked_sub(first).map(|age| age >= GRACE_SECONDS).unwrap_or(false)
}

// Called only with the network-fetched, signature-verified registry and the
// idle managed runtime's health/props. No network response = no retirement.
pub(crate) fn observe(
    registry: &VerifiedSignedModelRegistryV1, state: &NodeState,
    active: &str, active_path: &Path,
) -> Result<Option<RetirementCandidate>, String> {
    let Some(root) = managed_root() else { return Ok(None); };
    let payload = &registry.payload;
    let now = now_unix()?;
    if now >= payload.expires_at_unix || payload.issued_at_unix > now.saturating_add(300) {
        return Ok(None);
    }
    let previous = load_ledger(&root)?;
    let mut entries = Vec::new();
    let mut candidate = None;
    if !protected_company_or_training() {
        for rule in &payload.windows_model_retirements_v177 {
            if rule.replacement_model != active || now < rule.deprecated_at_unix { continue; }
            let (Some(old), Some(new)) = (definition(payload, &rule.retired_model),
                definition(payload, &rule.replacement_model)) else { continue; };
            if !certified(state, old) || !certified(state, new) { continue; }
            let artifact = &old.artifacts[0];
            let Some(owned) = read_receipt(&root, &artifact.filename, &artifact.sha256)
                else { continue; };
            let Some(replacement) = replacement_artifact(&root, new) else { continue; };
            if fs::canonicalize(active_path).ok() != fs::canonicalize(root.join(&replacement.filename)).ok()
                || !exact_discovery(&root, &old.selected_model, &owned.filename)
                || !exact_discovery(&root, active, &replacement.filename) { continue; }
            let first = previous.iter().find(|entry| continuous_observation(
                entry, rule, &owned, &replacement, now, payload.issued_at_unix
            )).map(|entry| entry.first_healthy_unix).unwrap_or(now);
            entries.push(Observation {
                rule: rule.clone(), replacement_sha256: replacement.sha256.clone(),
                owned: owned.clone(), first_healthy_unix: first, last_healthy_unix: now,
                last_registry_issued: payload.issued_at_unix,
            });
            if candidate.is_none() && grace_elapsed(first, now) {
                candidate = Some(RetirementCandidate {
                    retired_model: rule.retired_model.clone(), replacement_model: active.into(),
                    root: root.clone(), owned, replacement,
                    expires_at_unix: payload.expires_at_unix,
                });
            }
        }
    }
    if !previous.is_empty() || !entries.is_empty() { save_ledger(&root, &entries)?; }
    Ok(candidate)
}

impl RetirementCandidate {
    // This method is reached only after the backend's idle withdrawal
    // permit, and a second health check of the unchanged replacement.
    pub(crate) fn delete_after_withdrawal(self) -> Result<u64, String> {
        if managed_root().as_ref() != Some(&self.root)
            || now_unix()? >= self.expires_at_unix || protected_company_or_training()
            || self.retired_model == self.replacement_model
            || self.owned.filename.eq_ignore_ascii_case(&self.replacement.filename)
        { return Err("retirement_predelete_gate_failed".into()); }
        let path = self.root.join(&self.owned.filename);
        let replacement_path = self.root.join(&self.replacement.filename);
        if identity(&replacement_path, &self.replacement.filename, &self.replacement.sha256)
                .as_ref() != Some(&self.replacement)
            || read_receipt(&self.root, &self.owned.filename, &self.owned.sha256)
                .as_ref() != Some(&self.owned)
            || !sha256_file(&replacement_path)?.eq_ignore_ascii_case(&self.replacement.sha256)
            || !sha256_file(&path)?.eq_ignore_ascii_case(&self.owned.sha256)
            || read_receipt(&self.root, &self.owned.filename, &self.owned.sha256)
                .as_ref() != Some(&self.owned)
            || now_unix()? >= self.expires_at_unix
        { return Err("retirement_artifact_identity_changed".into()); }
        // Never recursively delete, remove a model directory, or touch certificates.
        fs::remove_file(&path).map_err(|e| format!("retirement_file_delete_failed:{e}"))?;
        let receipt = receipt_path(&self.root, &self.owned.filename);
        if plain_file(&receipt).is_some() { let _ = fs::remove_file(receipt); }
        Ok(self.owned.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grace_is_seven_full_days_and_clock_rollback_fails_closed() {
        assert!(!grace_elapsed(100, 99));
        assert!(!grace_elapsed(100, 100 + GRACE_SECONDS - 1));
        assert!(grace_elapsed(100, 100 + GRACE_SECONDS));
    }

    #[test]
    fn exact_model_filenames_only() {
        assert!(safe_filename("LFM3-1.7B-Q4_K_M.gguf"));
        for bad in ["../model.gguf", "a\\model.gguf", "C:model.gguf", ".hidden.gguf",
                    "model.gguf:stream", "model.gguf.exe", "model.json"] {
            assert!(!safe_filename(bad));
        }
    }

    #[test]
    fn observation_resets_on_policy_artifact_or_time_change() {
        let owned = OwnedArtifact { version: 1, filename: "old.gguf".into(),
            sha256: "a".repeat(64), bytes: 100, modified_secs: 1, modified_nanos: 0 };
        let replacement = OwnedArtifact { filename: "new.gguf".into(), sha256: "b".repeat(64),
            ..owned.clone() };
        let rule = WindowsModelRetirementV177 { retired_model: "old".into(),
            replacement_model: "new".into(), artifact_sha256: owned.sha256.clone(),
            deprecated_at_unix: 10 };
        let observation = Observation { rule: rule.clone(), owned: owned.clone(),
            replacement_sha256: replacement.sha256.clone(), first_healthy_unix: 20,
            last_healthy_unix: 30, last_registry_issued: 25 };
        assert!(continuous_observation(&observation, &rule, &owned, &replacement, 40, 35));
        assert!(!continuous_observation(&observation, &rule, &owned, &replacement, 29, 35));
        assert!(!continuous_observation(&observation, &rule, &owned, &replacement, 40, 24));
        assert!(!continuous_observation(&observation, &rule, &owned, &replacement,
            31 + MAX_OBSERVATION_GAP, 35));
        let changed = OwnedArtifact { modified_secs: 2, ..owned.clone() };
        assert!(!continuous_observation(&observation, &rule, &changed, &replacement, 40, 35));
        let changed_rule = WindowsModelRetirementV177 { deprecated_at_unix: 11, ..rule.clone() };
        assert!(!continuous_observation(&observation, &changed_rule, &owned, &replacement, 40, 35));
    }

    #[test]
    fn preexisting_or_modified_artifact_without_matching_receipt_is_not_owned() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = env::temp_dir().join(format!("swarm-retirement-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let name = "old.gguf";
        let path = root.join(name);
        fs::write(&path, b"fixture").unwrap();
        let sha = sha256_file(&path).unwrap();
        assert!(read_receipt(&root, name, &sha).is_none());
        let receipt = identity(&path, name, &sha).unwrap();
        fs::write(receipt_path(&root, name), serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert_eq!(read_receipt(&root, name, &sha), Some(receipt));
        fs::write(&path, b"changed fixture").unwrap();
        assert!(read_receipt(&root, name, &sha).is_none());
        fs::remove_dir_all(&root).unwrap();
    }
}
