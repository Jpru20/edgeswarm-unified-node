use crate::{adapters, core::certificate_match::sha256_file};
use reqwest::{
    blocking::Client,
    header::{CONTENT_RANGE, RANGE},
    StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io::copy;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadProgressV1 {
    pub status: String,
    pub filename: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
    pub bytes_per_second: f64,
    pub eta_seconds: Option<u64>,
}

static MODEL_DOWNLOAD_PROGRESS: OnceLock<Mutex<Option<ModelDownloadProgressV1>>> = OnceLock::new();
static MODEL_DOWNLOAD_LAST_PERCENT: AtomicU64 = AtomicU64::new(u64::MAX);

fn progress_store_v1() -> &'static Mutex<Option<ModelDownloadProgressV1>> {
    MODEL_DOWNLOAD_PROGRESS.get_or_init(|| Mutex::new(None))
}

pub fn model_download_progress_v1() -> Option<ModelDownloadProgressV1> {
    progress_store_v1().lock().ok().and_then(|v| v.clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelArtifactV1 {
    pub filename: String,
    pub download_url: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct ModelRecommendationV1 {
    pub model_id: String,
    pub capability: Option<String>,
    pub runtime: String,
    pub level: u64,
    pub should_download: bool,
    pub files: Vec<ModelArtifactV1>,
}

pub fn model_root_v1() -> PathBuf {
    env::var_os("EDGESWARM_MODEL_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let app_data = adapters::app_data_dir();

            #[cfg(target_os = "macos")]
            if let Some(parent) = app_data.parent() {
                return parent.join("models");
            }

            app_data.join("models")
        })
}

fn artifact_from_value(value: &Value) -> Result<ModelArtifactV1, String> {
    let filename = value
        .get("filename")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if filename.is_empty() || filename.contains('/') || filename.contains('\\') {
        return Err("invalid_model_filename".into());
    }

    let download_url = value
        .get("downloadUrl")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if !download_url.starts_with("https://") {
        return Err("invalid_model_download_url".into());
    }

    let sha256 = value
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("model_sha256_required".into());
    }

    Ok(ModelArtifactV1 {
        filename,
        download_url,
        sha256,
    })
}

pub fn parse_model_recommendation_v1(value: &Value) -> Result<ModelRecommendationV1, String> {
    let model = value
        .get("recommendedModel")
        .ok_or_else(|| "recommended_model_missing".to_string())?;

    let model_id = model
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if model_id.is_empty() {
        return Err("recommended_model_id_missing".into());
    }

    let mut files = Vec::new();

    if let Some(pack) = model.get("files").and_then(Value::as_array) {
        for file in pack {
            files.push(artifact_from_value(file)?);
        }
    }

    if files.is_empty() && model.get("filename").is_some() && model.get("downloadUrl").is_some() {
        files.push(artifact_from_value(model)?);
    }

    let should_download = value
        .get("shouldDownload")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let deterministic_only = model
        .get("capability")
        .and_then(Value::as_str)
        .map(|v| v.eq_ignore_ascii_case("Deterministic-Only"))
        .unwrap_or(false);

    if files.is_empty() && !(deterministic_only && !should_download) {
        return Err("recommended_model_files_missing".into());
    }

    Ok(ModelRecommendationV1 {
        model_id,
        capability: model
            .get("capability")
            .and_then(Value::as_str)
            .map(str::to_string),
        runtime: model
            .get("runtime")
            .and_then(Value::as_str)
            .unwrap_or("llama.cpp")
            .to_string(),
        level: model.get("level").and_then(Value::as_u64).unwrap_or(0),
        should_download,
        files,
    })
}

pub fn fetch_model_recommendation_v1(
    base_url: &str,
    hardware_payload: &Value,
) -> Result<ModelRecommendationV1, String> {
    let url = format!(
        "{}/node/model-recommendation",
        base_url.trim_end_matches('/')
    );

    let response = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("model_http_client_failed:{e}"))?
        .post(url)
        .json(hardware_payload)
        .send()
        .map_err(|e| format!("model_recommendation_request_failed:{e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "model_recommendation_http_{}",
            response.status().as_u16()
        ));
    }

    let value = response
        .json::<Value>()
        .map_err(|e| format!("model_recommendation_json_failed:{e}"))?;

    parse_model_recommendation_v1(&value)
}

pub fn verify_model_artifact_v1(root: &Path, artifact: &ModelArtifactV1) -> bool {
    let path = root.join(&artifact.filename);

    if !path.is_file() {
        return false;
    }

    sha256_file(&path)
        .map(|actual| actual.eq_ignore_ascii_case(&artifact.sha256))
        .unwrap_or(false)
}

fn update_download_progress_v1(
    filename: &str,
    downloaded: u64,
    total: u64,
    started: Instant,
    session_start: u64,
    status: &str,
) {
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let speed = downloaded.saturating_sub(session_start) as f64 / elapsed;
    let percent = if total == 0 {
        0.0
    } else {
        downloaded as f64 * 100.0 / total as f64
    };
    let eta = if speed > 1.0 {
        Some((total.saturating_sub(downloaded) as f64 / speed).ceil() as u64)
    } else {
        None
    };

    if let Ok(mut slot) = progress_store_v1().lock() {
        *slot = Some(ModelDownloadProgressV1 {
            status: status.into(),
            filename: filename.into(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            percent,
            bytes_per_second: speed,
            eta_seconds: eta,
        });
    }

    let whole = percent.floor().clamp(0.0, 100.0) as u64;
    if MODEL_DOWNLOAD_LAST_PERCENT.swap(whole, AtomicOrdering::Relaxed) != whole {
        println!(
            "MODEL_DOWNLOAD_PROGRESS={filename}|{downloaded}|{total}|{percent:.1}|{speed:.0}|{}",
            eta.unwrap_or(0)
        );
    }
}

fn remote_artifact_size_v1(client: &Client, artifact: &ModelArtifactV1) -> Result<u64, String> {
    let response = client
        .get(&artifact.download_url)
        .header(RANGE, "bytes=0-0")
        .send()
        .map_err(|e| format!("model_download_size_probe_failed:{e}"))?;

    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(format!(
            "model_download_size_probe_http_{}",
            response.status().as_u16()
        ));
    }

    let raw = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| "model_download_size_content_range_missing".to_string())?;

    let total = raw
        .strip_prefix("bytes 0-0/")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| format!("model_download_size_content_range_invalid:{raw}"))?;

    if total == 0 {
        return Err("model_download_size_zero".into());
    }

    Ok(total)
}

pub fn set_model_download_stage_v1(status: &str) {
    if let Ok(mut slot) = progress_store_v1().lock() {
        if let Some(progress) = slot.as_mut() {
            progress.status = status.to_string();
        }
    }
}

fn blocked_large_model_v1(recommendation: &ModelRecommendationV1) -> bool {
    recommendation.level >= 6
        || recommendation.model_id.to_ascii_lowercase().contains("70b")
        || recommendation
            .capability
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("70b")
}

fn validate_content_range_v1(
    response: &reqwest::blocking::Response,
    start: u64,
    end: u64,
    total: u64,
) -> Result<(), String> {
    let actual = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "model_range_content_range_missing".to_string())?;
    let expected = format!("bytes {start}-{end}/{total}");
    if actual.trim() != expected {
        return Err(format!(
            "model_range_content_range_mismatch:expected={expected}:actual={actual}"
        ));
    }
    let expected_len = end - start + 1;
    if response.content_length() != Some(expected_len) {
        return Err("model_range_content_length_mismatch".into());
    }
    Ok(())
}

fn download_range_part_v1(
    client: Client,
    artifact: ModelArtifactV1,
    path: PathBuf,
    start: u64,
    end: u64,
    progress: Arc<AtomicU64>,
    total: u64,
    session_start: u64,
    started: Instant,
) -> Result<(), String> {
    let expected = end - start + 1;
    for attempt in 1..=5u64 {
        let existing = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if existing == expected {
            return Ok(());
        }
        if existing > expected {
            let _ = fs::remove_file(&path);
            continue;
        }
        let from = start + existing;
        let result = (|| -> Result<(), String> {
            let mut response = client
                .get(&artifact.download_url)
                .header(RANGE, format!("bytes={from}-{end}"))
                .send()
                .map_err(|e| format!("model_range_request_failed:{e}"))?;
            if response.status() != StatusCode::PARTIAL_CONTENT {
                return Err(format!("model_range_http_{}", response.status().as_u16()));
            }
            validate_content_range_v1(&response, from, end, total)?;
            let mut out = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| format!("model_range_open_failed:{e}"))?;
            let mut written = existing;
            let mut buffer = vec![0u8; 1024 * 1024];
            while written < expected {
                let n = response
                    .read(&mut buffer)
                    .map_err(|e| format!("model_range_read_failed:{e}"))?;
                if n == 0 {
                    break;
                }
                let take = n.min((expected - written) as usize);
                out.write_all(&buffer[..take])
                    .map_err(|e| format!("model_range_write_failed:{e}"))?;
                written += take as u64;
                let done = progress.fetch_add(take as u64, AtomicOrdering::Relaxed) + take as u64;
                update_download_progress_v1(
                    &artifact.filename,
                    done,
                    total,
                    started,
                    session_start,
                    "downloading",
                );
            }
            out.flush()
                .map_err(|e| format!("model_range_flush_failed:{e}"))?;
            if written != expected {
                return Err("model_range_incomplete".into());
            }
            Ok(())
        })();
        if result.is_ok() {
            return result;
        }
        if attempt < 5 {
            thread::sleep(Duration::from_secs((attempt * 2).min(10)));
        }
    }
    Err("model_range_retries_exhausted".into())
}

fn assemble_verified_model_v1(
    root: &Path,
    artifact: &ModelArtifactV1,
    prefix: &Path,
    parts: &[PathBuf],
    total: u64,
    started: Instant,
    session_start: u64,
) -> Result<PathBuf, String> {
    let final_path = root.join(&artifact.filename);
    let assembly = root.join(format!("{}.download.assembled", artifact.filename));
    let _ = fs::remove_file(&assembly);
    let mut out =
        File::create(&assembly).map_err(|e| format!("model_assembly_create_failed:{e}"))?;
    if prefix.is_file() {
        let mut input = File::open(prefix).map_err(|e| format!("model_prefix_open_failed:{e}"))?;
        copy(&mut input, &mut out).map_err(|e| format!("model_prefix_copy_failed:{e}"))?;
    }
    for part in parts {
        let mut input = File::open(part).map_err(|e| format!("model_part_open_failed:{e}"))?;
        copy(&mut input, &mut out).map_err(|e| format!("model_part_copy_failed:{e}"))?;
    }
    out.flush()
        .map_err(|e| format!("model_assembly_flush_failed:{e}"))?;
    if fs::metadata(&assembly).map(|m| m.len()).unwrap_or(0) != total {
        return Err("model_assembly_size_mismatch".into());
    }
    update_download_progress_v1(
        &artifact.filename,
        total,
        total,
        started,
        session_start,
        "verifying",
    );
    let actual = sha256_file(&assembly)?;
    if !actual.eq_ignore_ascii_case(&artifact.sha256) {
        set_model_download_stage_v1("error");
        let _ = fs::remove_file(&assembly);
        let _ = fs::remove_file(prefix);
        for part in parts {
            let _ = fs::remove_file(part);
        }
        let manifest = root.join(format!("{}.download.sha256", artifact.filename));
        let _ = fs::remove_file(manifest);
        return Err(format!(
            "model_sha256_mismatch:expected={}:actual={}",
            artifact.sha256, actual
        ));
    }
    fs::rename(&assembly, &final_path).map_err(|e| format!("model_atomic_install_failed:{e}"))?;
    let _ = fs::remove_file(prefix);
    for part in parts {
        let _ = fs::remove_file(part);
    }
    update_download_progress_v1(
        &artifact.filename,
        total,
        total,
        started,
        session_start,
        "installed",
    );
    Ok(final_path)
}

fn download_artifact_v1(
    client: &Client,
    root: &Path,
    artifact: &ModelArtifactV1,
) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|e| format!("model_directory_failed:{e}"))?;
    if verify_model_artifact_v1(root, artifact) {
        return Ok(root.join(&artifact.filename));
    }

    let prefix = root.join(format!("{}.download", artifact.filename));
    let manifest = root.join(format!("{}.download.sha256", artifact.filename));
    let assembly = root.join(format!("{}.download.assembled", artifact.filename));

    let checkpoint_valid = fs::read_to_string(&manifest)
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case(&artifact.sha256))
        == Some(true);
    let legacy_checkpoint = !manifest.exists() && prefix.is_file();

    if legacy_checkpoint {
        fs::write(&manifest, &artifact.sha256)
            .map_err(|e| format!("model_checkpoint_manifest_failed:{e}"))?;
    } else if !checkpoint_valid {
        let _ = fs::remove_file(&prefix);
        let _ = fs::remove_file(&assembly);
        for i in 0..8 {
            let _ = fs::remove_file(root.join(format!("{}.download.part-{i}", artifact.filename)));
        }
        fs::write(&manifest, &artifact.sha256)
            .map_err(|e| format!("model_checkpoint_manifest_failed:{e}"))?;
    }

    let total = remote_artifact_size_v1(client, artifact)?;
    if total == 0 {
        return Err("model_download_size_zero".into());
    }
    let mut prefix_len = fs::metadata(&prefix).map(|m| m.len()).unwrap_or(0);
    if prefix_len > total {
        let _ = fs::remove_file(&prefix);
        prefix_len = 0;
    }

    let remaining = total.saturating_sub(prefix_len);
    let workers = if remaining == 0 {
        0
    } else {
        4u64.min(remaining)
    };
    let chunk = if workers == 0 {
        0
    } else {
        (remaining + workers - 1) / workers
    };
    let mut parts = Vec::new();

    for i in 0..workers {
        let start = prefix_len + i * chunk;
        if start >= total {
            break;
        }
        let end = (start + chunk - 1).min(total - 1);
        let path = root.join(format!("{}.download.part-{i}", artifact.filename));
        let expected = end - start + 1;
        if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > expected {
            let _ = fs::remove_file(&path);
        }
        parts.push((path, start, end));
    }

    let session_start = prefix_len
        + parts
            .iter()
            .map(|(p, _, _)| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum::<u64>();
    let progress = Arc::new(AtomicU64::new(session_start));
    let started = Instant::now();
    update_download_progress_v1(
        &artifact.filename,
        session_start,
        total,
        started,
        session_start,
        "downloading",
    );

    let mut handles = Vec::new();
    for (path, start, end) in parts.iter().cloned() {
        let c = client.clone();
        let a = artifact.clone();
        let pr = progress.clone();
        handles.push(thread::spawn(move || {
            download_range_part_v1(c, a, path, start, end, pr, total, session_start, started)
        }));
    }
    for handle in handles {
        handle
            .join()
            .map_err(|_| "model_range_worker_panicked".to_string())??;
    }

    let ordered = parts.into_iter().map(|(p, _, _)| p).collect::<Vec<_>>();
    let installed = assemble_verified_model_v1(
        root,
        artifact,
        &prefix,
        &ordered,
        total,
        started,
        session_start,
    )?;
    let _ = fs::remove_file(&manifest);
    Ok(installed)
}

pub fn provision_recommendation_v1(
    recommendation: &ModelRecommendationV1,
) -> Result<Vec<PathBuf>, String> {
    if blocked_large_model_v1(recommendation) {
        return Err("large_model_auto_download_blocked".into());
    }

    let root = model_root_v1();

    if !recommendation.should_download {
        let ready = recommendation
            .files
            .iter()
            .all(|file| verify_model_artifact_v1(&root, file));

        if !ready {
            return Err("recommended_model_not_ready".into());
        }

        return Ok(recommendation
            .files
            .iter()
            .map(|file| root.join(&file.filename))
            .collect());
    }

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(7200))
        .build()
        .map_err(|e| format!("model_download_client_failed:{e}"))?;

    let mut paths = Vec::new();

    for artifact in &recommendation.files {
        paths.push(download_artifact_v1(&client, &root, artifact)?);
    }

    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_multi_file_pack_and_requires_sha() {
        let sha = "a".repeat(64);

        let value = json!({
            "shouldDownload": true,
            "recommendedModel": {
                "id": "qwen2.5:14b",
                "capability": "Neural-Inference-14B",
                "runtime": "llama.cpp",
                "files": [
                    {
                        "filename": "general.gguf",
                        "downloadUrl": "https://api.edgeswarm.io/models/general.gguf",
                        "sha256": sha
                    },
                    {
                        "filename": "coder.gguf",
                        "downloadUrl": "https://api.edgeswarm.io/models/coder.gguf",
                        "sha256": "b".repeat(64)
                    }
                ]
            }
        });

        let parsed = parse_model_recommendation_v1(&value).unwrap();

        assert_eq!(parsed.files.len(), 2);
        assert!(parsed.should_download);
    }

    #[test]
    fn accepts_deterministic_recommendation_without_files() {
        let value = json!({
            "shouldDownload": false,
            "recommendedModel": {
                "id": "edgeswarm-level1-deterministic",
                "capability": "Deterministic-Only",
                "level": 1,
                "files": []
            }
        });

        let parsed = parse_model_recommendation_v1(&value).unwrap();
        assert!(parsed.files.is_empty());
        assert!(!parsed.should_download);
    }

    #[test]
    fn rejects_missing_sha_and_unsafe_filename() {
        let missing_sha = json!({
            "recommendedModel": {
                "id": "qwen2.5:3b",
                "filename": "model.gguf",
                "downloadUrl": "https://api.edgeswarm.io/models/model.gguf"
            }
        });

        assert!(parse_model_recommendation_v1(&missing_sha).is_err());

        let unsafe_name = json!({
            "recommendedModel": {
                "id": "qwen2.5:3b",
                "filename": "../model.gguf",
                "downloadUrl": "https://api.edgeswarm.io/models/model.gguf",
                "sha256": "a".repeat(64)
            }
        });

        assert!(parse_model_recommendation_v1(&unsafe_name).is_err());
    }
}
