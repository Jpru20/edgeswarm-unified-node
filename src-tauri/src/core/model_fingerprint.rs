use crate::{
    adapters,
    core::certificate_match::sha256_file,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use uuid::Uuid;

const CACHE_VERSION: &str = "edgeswarm-model-fingerprint-cache-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FingerprintEntryV1 {
    selected_model: String,
    canonical_path: String,
    file_size: u64,
    modified_unix_ns: u128,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FingerprintCacheV1 {
    version: String,
    entries: Vec<FingerprintEntryV1>,
}

#[derive(Debug, Clone)]
pub struct ResolvedFingerprintV1 {
    pub selected_model: String,
    pub canonical_path: String,
    pub file_size: u64,
    pub sha256: String,
    pub cache_hit: bool,
}

impl Default for FingerprintCacheV1 {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION.into(),
            entries: Vec::new(),
        }
    }
}

fn cache_path() -> Result<PathBuf, String> {
    let identity = adapters::identity_file_path();

    let parent = identity
        .parent()
        .ok_or_else(|| "identity_parent_missing".to_string())?;

    Ok(parent.join("model_fingerprints_v1.json"))
}

fn file_stamp(path: &Path) -> Result<(u64, u128), String> {
    let metadata = fs::metadata(path)
        .map_err(|e| format!("model_metadata_failed:{e}"))?;

    if !metadata.is_file() {
        return Err("model_path_not_file".into());
    }

    let modified = metadata
        .modified()
        .map_err(|e| format!("model_modified_failed:{e}"))?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("model_modified_invalid:{e}"))?
        .as_nanos();

    Ok((metadata.len(), modified))
}

fn load_cache(path: &Path) -> FingerprintCacheV1 {
    let Ok(raw) = fs::read_to_string(path) else {
        return FingerprintCacheV1::default();
    };

    let Ok(cache) = serde_json::from_str::<FingerprintCacheV1>(&raw) else {
        return FingerprintCacheV1::default();
    };

    if cache.version != CACHE_VERSION {
        return FingerprintCacheV1::default();
    }

    cache
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn save_cache(
    path: &Path,
    cache: &FingerprintCacheV1,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "cache_parent_missing".to_string())?;

    fs::create_dir_all(parent)
        .map_err(|e| format!("cache_directory_failed:{e}"))?;

    let raw = serde_json::to_vec_pretty(cache)
        .map_err(|e| format!("cache_serialize_failed:{e}"))?;

    let temporary = parent.join(format!(
        ".model-fingerprint-{}.tmp",
        Uuid::new_v4()
    ));

    fs::write(&temporary, raw)
        .map_err(|e| format!("cache_write_failed:{e}"))?;

    fs::rename(&temporary, path)
        .map_err(|e| format!("cache_commit_failed:{e}"))?;

    Ok(())
}

fn resolve_with_cache(
    cache_file: &Path,
    selected_model: &str,
    model_path: &Path,
    force_rehash: bool,
) -> Result<ResolvedFingerprintV1, String> {
    let canonical = fs::canonicalize(model_path)
        .map_err(|e| format!("model_canonicalize_failed:{e}"))?;

    let canonical_text = canonical.to_string_lossy().to_string();

    let (size_before, modified_before) = file_stamp(&canonical)?;
    let mut cache = load_cache(cache_file);

    if !force_rehash {
        if let Some(entry) = cache.entries.iter().find(|entry| {
            entry.selected_model == selected_model
                && entry.canonical_path == canonical_text
                && entry.file_size == size_before
                && entry.modified_unix_ns == modified_before
                && valid_sha256(&entry.sha256)
        }) {
            return Ok(ResolvedFingerprintV1 {
                selected_model: selected_model.into(),
                canonical_path: canonical_text,
                file_size: size_before,
                sha256: entry.sha256.clone(),
                cache_hit: true,
            });
        }
    }

    let sha256 = sha256_file(&canonical)?;

    let (size_after, modified_after) = file_stamp(&canonical)?;

    if size_before != size_after || modified_before != modified_after {
        return Err("model_changed_during_hash".into());
    }

    cache.entries.retain(|entry| {
        !(entry.selected_model == selected_model
            && entry.canonical_path == canonical_text)
    });

    cache.entries.push(FingerprintEntryV1 {
        selected_model: selected_model.into(),
        canonical_path: canonical_text.clone(),
        file_size: size_after,
        modified_unix_ns: modified_after,
        sha256: sha256.clone(),
    });

    save_cache(cache_file, &cache)?;

    Ok(ResolvedFingerprintV1 {
        selected_model: selected_model.into(),
        canonical_path: canonical_text,
        file_size: size_after,
        sha256,
        cache_hit: false,
    })
}

pub fn resolve_model_fingerprint(
    selected_model: &str,
    model_path: &Path,
    force_rehash: bool,
) -> Result<ResolvedFingerprintV1, String> {
    resolve_with_cache(
        &cache_path()?,
        selected_model,
        model_path,
        force_rehash,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn fingerprint_cache_reuses_unchanged_artifact() {
        let root = env::temp_dir().join(format!(
            "edgeswarm-fingerprint-test-{}",
            Uuid::new_v4()
        ));

        fs::create_dir_all(&root).unwrap();

        let model = root.join("model.gguf");
        let cache = root.join("cache.json");

        fs::write(&model, b"abc").unwrap();

        let first = resolve_with_cache(
            &cache,
            "test:model",
            &model,
            false,
        ).unwrap();

        assert!(!first.cache_hit);

        let second = resolve_with_cache(
            &cache,
            "test:model",
            &model,
            false,
        ).unwrap();

        assert!(second.cache_hit);
        assert_eq!(first.sha256, second.sha256);

        fs::write(&model, b"abcdef").unwrap();

        let changed = resolve_with_cache(
            &cache,
            "test:model",
            &model,
            false,
        ).unwrap();

        assert!(!changed.cache_hit);
        assert_ne!(second.sha256, changed.sha256);

        let _ = fs::remove_dir_all(root);
    }
}
