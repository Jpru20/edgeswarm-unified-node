use crate::{adapters, core::certificate_match::sha256_file};
use reqwest::{blocking::Client, header::RANGE, StatusCode};
use serde_json::Value;
use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

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

    if files.is_empty() {
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
        should_download: value
            .get("shouldDownload")
            .and_then(Value::as_bool)
            .unwrap_or(false),
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

fn download_artifact_v1(
    client: &Client,
    root: &Path,
    artifact: &ModelArtifactV1,
) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|e| format!("model_directory_failed:{e}"))?;

    let final_path = root.join(&artifact.filename);
    let temp_path = root.join(format!("{}.download", artifact.filename));

    if verify_model_artifact_v1(root, artifact) {
        return Ok(final_path);
    }

    if final_path.exists() {
        fs::remove_file(&final_path).map_err(|e| format!("invalid_model_remove_failed:{e}"))?;
    }

    let mut last_error = "model_download_failed".to_string();

    for attempt in 1..=5u64 {
        let result = (|| -> Result<PathBuf, String> {
            let resume_from = fs::metadata(&temp_path).map(|m| m.len()).unwrap_or(0);

            let mut request = client.get(&artifact.download_url);

            if resume_from > 0 {
                request = request.header(RANGE, format!("bytes={resume_from}-"));
            }

            let mut response = request
                .send()
                .map_err(|e| format!("model_download_request_failed:{e}"))?;

            if response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
                let _ = fs::remove_file(&temp_path);
                return Err("model_resume_range_rejected".into());
            }

            if resume_from > 0 && response.status() == StatusCode::OK {
                let _ = fs::remove_file(&temp_path);
                return Err("model_resume_ignored_restart".into());
            }

            if response.status() != StatusCode::OK
                && response.status() != StatusCode::PARTIAL_CONTENT
            {
                return Err(format!(
                    "model_download_http_{}",
                    response.status().as_u16()
                ));
            }

            let mut output = OpenOptions::new()
                .create(true)
                .write(true)
                .append(resume_from > 0)
                .truncate(resume_from == 0)
                .open(&temp_path)
                .map_err(|e| format!("model_temp_open_failed:{e}"))?;

            let mut buffer = vec![0u8; 1024 * 1024];

            loop {
                let count = response
                    .read(&mut buffer)
                    .map_err(|e| format!("model_download_read_failed:{e}"))?;

                if count == 0 {
                    break;
                }

                output
                    .write_all(&buffer[..count])
                    .map_err(|e| format!("model_download_write_failed:{e}"))?;
            }

            output
                .flush()
                .map_err(|e| format!("model_download_flush_failed:{e}"))?;

            let actual = sha256_file(&temp_path)?;

            if !actual.eq_ignore_ascii_case(&artifact.sha256) {
                let _ = fs::remove_file(&temp_path);
                return Err("model_sha256_mismatch".into());
            }

            fs::rename(&temp_path, &final_path)
                .map_err(|e| format!("model_atomic_install_failed:{e}"))?;

            Ok(final_path.clone())
        })();

        match result {
            Ok(path) => return Ok(path),
            Err(error) => {
                last_error = error;

                if attempt < 5 {
                    thread::sleep(Duration::from_secs((attempt * 2).min(10)));
                }
            }
        }
    }

    Err(last_error)
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
