//! Windows capacity-policy upgrade: preserve valid v1 certificates until a
//! successful idle-time v2 replacement, with persistent failure backoff.
use crate::core::{
    capacity_policy::{CAPACITY_POLICY_VERSION_V1, CAPACITY_POLICY_VERSION_V2},
    model::ModelState,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

const RETRY_COOLDOWN_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MigrationRecord {
    attempted_at_unix_ms: u64,
    last_result: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MigrationJournal {
    #[serde(default)]
    models: BTreeMap<String, MigrationRecord>,
}

fn path() -> PathBuf {
    crate::adapters::app_data_dir().join("capacity_policy_migration_v175.json")
}

fn read() -> Result<MigrationJournal, String> {
    let path = path();
    if !path.exists() {
        return Ok(MigrationJournal::default());
    }
    let bytes = fs::read(path).map_err(|_| "capacity_migration_read_failed".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "capacity_migration_journal_invalid".to_string())
}

fn write(journal: &MigrationJournal) -> Result<(), String> {
    let path = path();
    let directory = path.parent().ok_or("capacity_migration_parent_missing")?;
    fs::create_dir_all(directory).map_err(|_| "capacity_migration_mkdir_failed".to_string())?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(journal)
        .map_err(|_| "capacity_migration_serialize_failed".to_string())?;
    fs::write(&temporary, bytes).map_err(|_| "capacity_migration_write_failed".to_string())?;
    // Preserve an existing readable journal until its replacement is ready.
    if let Err(error) = fs::rename(&temporary, &path) {
        #[cfg(target_os = "windows")]
        if path.exists() {
            // On Windows rename cannot replace an existing file. Keep the
            // previous journal if this replacement cannot be committed.
            let backup = path.with_extension("bak");
            fs::rename(&path, &backup)
                .map_err(|_| "capacity_migration_backup_failed".to_string())?;
            if fs::rename(&temporary, &path).is_err() {
                let _ = fs::rename(&backup, &path);
                return Err("capacity_migration_commit_failed".into());
            }
            let _ = fs::remove_file(backup);
            return Ok(());
        }
        return Err(format!("capacity_migration_commit_failed:{error}"));
    }
    Ok(())
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn pending_model_at(
    models: &[ModelState],
    attempts: &BTreeMap<String, (u64, String)>,
    now_ms: u64,
) -> Option<String> {
    models.iter().filter(|m| {
        m.status == "ready"
            && m.capacity_policy_version.as_deref() == Some(CAPACITY_POLICY_VERSION_V1)
    }).find(|m| {
        attempts.get(&m.selected_model).map(|(last, _)| {
            now_ms.saturating_sub(*last) >= RETRY_COOLDOWN_MS
        }).unwrap_or(true)
    }).map(|m| m.selected_model.clone())
}

pub fn next_pending_model(models: &[ModelState]) -> Result<Option<String>, String> {
    let journal = read()?;
    let attempts = journal.models.iter().map(|(name, r)| {
        (name.clone(), (r.attempted_at_unix_ms, r.last_result.clone()))
    }).collect();
    Ok(pending_model_at(models, &attempts, now_unix_ms()))
}

pub fn record_attempt(selected_model: &str, result: &str) -> Result<(), String> {
    if !matches!(result, "started" | "pass" | "failed") {
        return Err("capacity_migration_result_invalid".into());
    }
    let mut journal = read()?;
    let timestamp = now_unix_ms();
    journal.models.insert(selected_model.to_string(), MigrationRecord {
        attempted_at_unix_ms: timestamp,
        last_result: result.to_string(),
    });
    write(&journal)
}

pub fn migration_status(selected_model: &str, policy: Option<&str>) -> &'static str {
    match policy {
        Some(CAPACITY_POLICY_VERSION_V2) => "certified_v2",
        Some(CAPACITY_POLICY_VERSION_V1) => {
            match read().ok().and_then(|j| j.models.get(selected_model).cloned()) {
                Some(record) if record.last_result == "failed" => "retry_after_cooldown",
                Some(record) if record.last_result == "started" => "scheduled_or_interrupted",
                _ => "pending_idle_recertification",
            }
        }
        _ => "uncertified_or_unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::capacity::CapacityStatus;

    fn model(policy: Option<&str>) -> ModelState {
        ModelState {
            selected_model: "lfm3:1.7b".into(),
            model_id: "LFM3".into(),
            capability: "Neural-Inference-3B".into(),
            tier: 2,
            runtime: "llama.cpp".into(),
            acceleration: "vulkan".into(),
            status: "ready".into(),
            capacity_status: CapacityStatus::Certified,
            certified_concurrency: Some(1),
            capacity_policy_version: policy.map(str::to_string),
        }
    }

    #[test]
    fn old_certificate_waits_for_idle_retest() {
        assert_eq!(pending_model_at(&[model(Some(CAPACITY_POLICY_VERSION_V1))],
            &BTreeMap::new(), 100), Some("lfm3:1.7b".into()));
    }
    #[test]
    fn certified_v2_never_retests() {
        assert_eq!(pending_model_at(&[model(Some(CAPACITY_POLICY_VERSION_V2))],
            &BTreeMap::new(), 100), None);
    }
    #[test]
    fn failed_attempt_has_persistent_cooldown() {
        let mut prior = BTreeMap::new();
        prior.insert("lfm3:1.7b".into(), (100, "failed".into()));
        let models = [model(Some(CAPACITY_POLICY_VERSION_V1))];
        assert_eq!(pending_model_at(&models, &prior, 200), None);
        assert_eq!(pending_model_at(&models, &prior, 100 + RETRY_COOLDOWN_MS), Some("lfm3:1.7b".into()));
    }
    #[test]
    fn incomplete_or_missing_certificate_is_not_migration_target() {
        assert_eq!(pending_model_at(&[model(None)], &BTreeMap::new(), 100), None);
    }
}
