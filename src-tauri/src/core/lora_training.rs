use crate::core::{
    capacity::CapacityStatus,
    NodeState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub const LORA_TRAINING_CAPABILITY_V1: &str =
    "LoRA-Training-V1";

#[cfg(target_os = "windows")]
pub const LORA_TRAINING_ENGINE_V1: &str = "edgeswarm-qlora-windows-v1";

#[cfg(target_os = "linux")]
pub const LORA_TRAINING_ENGINE_V1: &str = "edgeswarm-qlora-linux-cuda-v1";

#[cfg(target_os = "macos")]
pub const LORA_TRAINING_ENGINE_V1: &str = "edgeswarm-mlx-macos-metal-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoraTrainingRuntimeManifestV1 {
    pub schema_version: u8,
    pub engine: String,
    pub status: String,
    pub trainer_sha256: String,

    #[serde(default)]
    pub supported_base_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoraTrainingStatusV1 {
    pub enabled: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_model_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_capability: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub trainer_sha256: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub training_memory_bytes: Option<u64>,

    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoraTrainingJobV1 {
    pub id: String,
    pub company_model_id: String,
    pub target_version: u64,
    pub base_model_id: String,
    pub base_capability: String,
    pub dataset_id: String,
    pub dataset_sha256: String,
    pub dataset_download_url: String,
    pub adapter_upload_url: String,
    pub adapter_artifact_uri: String,

    #[serde(default)]
    pub hyperparameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoraTrainingProgressV1 {
    pub phase: String,

    #[serde(default)]
    pub step: Option<u64>,

    #[serde(default)]
    pub total_steps: Option<u64>,

    #[serde(default)]
    pub epoch: Option<f64>,

    #[serde(default)]
    pub loss: Option<f64>,

    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoraTrainingOutputV1 {
    pub adapter_path: PathBuf,
    pub adapter_sha256: String,
    pub metrics: Value,
}

fn disabled(
    reason: impl Into<String>,
) -> LoraTrainingStatusV1 {
    LoraTrainingStatusV1 {
        enabled: false,
        base_model_id: None,
        base_capability: None,
        engine: None,
        trainer_sha256: None,
        training_memory_bytes: None,
        reason: reason.into(),
    }
}

pub fn training_runtime_root_v1() -> Option<PathBuf> {
    if let Some(value) =
        env::var_os("EDGESWARM_LORA_TRAINING_RUNTIME")
    {
        let path = PathBuf::from(value);

        if path.is_dir() {
            return Some(path);
        }
    }

    let executable =
        env::current_exe().ok()?;

    let install_root =
        executable.parent()?;

    [
        install_root
            .join("resources")
            .join("training")
            .join("current"),

        install_root
            .join("training")
            .join("current"),
    ]
    .into_iter()
    .find(|path| path.is_dir())
}

pub fn trainer_executable_v1(
    root: &Path,
) -> PathBuf {
    root.join(
        if cfg!(target_os = "windows") {
            "edgeswarm-lora-trainer.exe"
        } else {
            "edgeswarm-lora-trainer"
        }
    )
}

fn trainer_manifest_path_v1(
    root: &Path,
) -> PathBuf {
    root.join(
        "trainer-manifest.json"
    )
}

pub fn sha256_file_v1(
    path: &Path,
) -> Result<String, String> {
    let mut file =
        fs::File::open(path)
            .map_err(|_| {
                "sha256_file_open_failed"
                    .to_string()
            })?;

    let mut digest =
        Sha256::new();

    let mut buffer =
        [0u8; 1024 * 1024];

    loop {
        let read =
            file.read(&mut buffer)
                .map_err(|_| {
                    "sha256_file_read_failed"
                        .to_string()
                })?;

        if read == 0 {
            break;
        }

        digest.update(
            &buffer[..read]
        );
    }

    Ok(
        digest
            .finalize()
            .iter()
            .map(|byte| {
                format!("{byte:02x}")
            })
            .collect::<String>()
    )
}

fn valid_sha256_v1(
    value: &str,
) -> bool {
    value.len() == 64 &&
        value
            .chars()
            .all(|c| c.is_ascii_hexdigit())
}

fn supported_base_model_v1(
    manifest: &LoraTrainingRuntimeManifestV1,
    model_id: &str,
) -> bool {
    manifest
        .supported_base_models
        .iter()
        .any(|value| {
            value
                .trim()
                .eq_ignore_ascii_case(
                    model_id.trim()
                )
        })
}

fn minimum_training_vram_bytes_v1(
    model_id: &str,
) -> Option<u64> {
    const GIB: u64 =
        1024 * 1024 * 1024;

    match model_id
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "qwen2.5:3b" =>
            Some(8 * GIB),

        "qwen2.5:7b" =>
            Some(12 * GIB),

        "qwen2.5:14b" =>
            Some(24 * GIB),

        _ =>
            None,
    }
}

pub fn detect_lora_training_v1(
    state: &NodeState,
    primary_model_id: Option<&str>,
) -> LoraTrainingStatusV1 {
    // HIVE_CROSS_PLATFORM_TRAINER_ELIGIBILITY_V1
    // Different native trainer artifacts have distinct pinned engines.
    // No platform is eligible without its own certified manifest and hash.
    let backend = state.acceleration.backend.trim();
    let expected_backend = if cfg!(target_os = "macos") {
        "metal"
    } else if cfg!(any(target_os = "windows", target_os = "linux")) {
        "cuda"
    } else {
        return disabled("platform_training_backend_unsupported");
    };

    if !backend.eq_ignore_ascii_case(expected_backend) {
        return disabled(format!("training_backend_required:{expected_backend}"));
    }

    {
        let model_id =
            match primary_model_id {
                Some(value)
                    if !value
                        .trim()
                        .is_empty() =>
                {
                    value.trim()
                        .to_string()
                }

                _ => {
                    return disabled(
                        "primary_model_missing"
                    );
                }
            };

        let model =
            match state
                .models
                .iter()
                .find(|model| {
                    model.selected_model
                        .eq_ignore_ascii_case(
                            &model_id
                        ) &&
                    model.status ==
                        "ready" &&
                    model.capacity_status ==
                        CapacityStatus::Certified
                })
            {
                Some(value) =>
                    value,

                None => {
                    return disabled(
                        "base_model_not_certified"
                    );
                }
            };

        let minimum_vram =
            match minimum_training_vram_bytes_v1(
                &model_id
            ) {
                Some(value) =>
                    value,

                None => {
                    return disabled(
                        "base_model_not_supported_for_lora_v1"
                    );
                }
            };

        let available_vram =
            match state
                .acceleration
                .vram_bytes
            {
                Some(value) =>
                    value,

                None => {
                    return disabled(
                        "gpu_memory_unknown"
                    );
                }
            };

        if available_vram <
            minimum_vram
        {
            return disabled(
                format!(
                    "insufficient_training_vram:{}<{}",
                    available_vram,
                    minimum_vram
                )
            );
        }

        let runtime_root =
            match training_runtime_root_v1() {
                Some(value) =>
                    value,

                None => {
                    return disabled(
                        "lora_training_runtime_missing"
                    );
                }
            };

        let trainer =
            trainer_executable_v1(
                &runtime_root
            );

        if !trainer.is_file() {
            return disabled(
                "lora_trainer_missing"
            );
        }

        let manifest_path =
            trainer_manifest_path_v1(
                &runtime_root
            );

        let manifest =
            match fs::read_to_string(
                &manifest_path
            )
            .ok()
            .and_then(|raw| {
                serde_json::from_str::<
                    LoraTrainingRuntimeManifestV1
                >(&raw)
                .ok()
            }) {
                Some(value) =>
                    value,

                None => {
                    return disabled(
                        "lora_trainer_manifest_invalid"
                    );
                }
            };

        if manifest.schema_version != 1 ||
            !manifest
                .engine
                .eq_ignore_ascii_case(
                    LORA_TRAINING_ENGINE_V1
                ) ||
            !manifest
                .status
                .eq_ignore_ascii_case(
                    "certified"
                )
        {
            return disabled(
                "lora_trainer_not_certified"
            );
        }

        if !valid_sha256_v1(
            &manifest.trainer_sha256
        ) {
            return disabled(
                "lora_trainer_manifest_hash_invalid"
            );
        }

        if !supported_base_model_v1(
            &manifest,
            &model_id
        ) {
            return disabled(
                "lora_trainer_base_model_unsupported"
            );
        }

        let actual_hash =
            match sha256_file_v1(
                &trainer
            ) {
                Ok(value) =>
                    value,

                Err(_) => {
                    return disabled(
                        "lora_trainer_hash_failed"
                    );
                }
            };

        if !actual_hash
            .eq_ignore_ascii_case(
                &manifest.trainer_sha256
            )
        {
            return disabled(
                "lora_trainer_hash_mismatch"
            );
        }

        LoraTrainingStatusV1 {
            enabled: true,

            base_model_id:
                Some(
                    model.selected_model
                        .clone()
                ),

            base_capability:
                Some(
                    model.capability
                        .clone()
                ),

            engine:
                Some(
                    LORA_TRAINING_ENGINE_V1
                        .into()
                ),

            trainer_sha256:
                Some(
                    actual_hash
                ),

            training_memory_bytes:
                Some(
                    available_vram
                ),

            reason:
                format!("certified_native_lora_runtime:{expected_backend}"),
        }
    }
}

fn parse_progress_line_v1(
    line: &str,
) -> Option<LoraTrainingProgressV1> {
    let value =
        serde_json::from_str::<Value>(
            line
        )
        .ok()?;

    if value
        .get("event")
        .and_then(Value::as_str) !=
        Some("progress")
    {
        return None;
    }

    Some(
        LoraTrainingProgressV1 {
            phase:
                value
                    .get("phase")
                    .and_then(Value::as_str)
                    .unwrap_or(
                        "training"
                    )
                    .to_string(),

            step:
                value
                    .get("step")
                    .and_then(Value::as_u64),

            total_steps:
                value
                    .get("totalSteps")
                    .and_then(Value::as_u64),

            epoch:
                value
                    .get("epoch")
                    .and_then(Value::as_f64),

            loss:
                value
                    .get("loss")
                    .and_then(Value::as_f64),

            message:
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string),
        }
    )
}

struct LoraTrainerChildGuardV1(std::process::Child);

impl std::ops::Deref for LoraTrainerChildGuardV1 {
    type Target = std::process::Child;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for LoraTrainerChildGuardV1 {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for LoraTrainerChildGuardV1 {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
pub fn run_lora_training_job_v1<F>(
    job: &LoraTrainingJobV1,
    dataset_path: &Path,
    output_dir: &Path,
    stop: &AtomicBool,
    mut on_progress: F,
) -> Result<LoraTrainingOutputV1, String>
where
    F: FnMut(
        &LoraTrainingProgressV1
    ) -> Result<(), String>,
{
    let root =
        training_runtime_root_v1()
            .ok_or_else(|| {
                "lora_training_runtime_missing"
                    .to_string()
            })?;

    let trainer =
        trainer_executable_v1(
            &root
        );

    if !trainer.is_file() {
        return Err(
            "lora_trainer_missing"
                .into()
        );
    }

    if !dataset_path.is_file() {
        return Err(
            "lora_dataset_missing"
                .into()
        );
    }

    fs::create_dir_all(
        output_dir
    )
    .map_err(|_| {
        "lora_output_directory_failed"
            .to_string()
    })?;

    let job_spec_path =
        output_dir.join(
            "job-spec.json"
        );

    let adapter_path =
        output_dir.join(
            "adapter.gguf"
        );

    let metrics_path =
        output_dir.join(
            "metrics.json"
        );

    let safe_job_spec =
        serde_json::json!({
            "schemaVersion": 1,
            "jobId": job.id,
            "companyModelId":
                job.company_model_id,
            "targetVersion":
                job.target_version,
            "baseModelId":
                job.base_model_id,
            "baseCapability":
                job.base_capability,
            "datasetId":
                job.dataset_id,
            "datasetSha256":
                job.dataset_sha256,
            "hyperparameters":
                job.hyperparameters
        });

    fs::write(
        &job_spec_path,
        serde_json::to_vec_pretty(
            &safe_job_spec
        )
        .map_err(|_| {
            "lora_job_spec_encode_failed"
                .to_string()
        })?,
    )
    .map_err(|_| {
        "lora_job_spec_write_failed"
            .to_string()
    })?;

    let mut command =
        Command::new(
            trainer
        );

    command
        .arg("--job-spec")
        .arg(&job_spec_path)
        .arg("--dataset")
        .arg(dataset_path)
        .arg("--adapter-output")
        .arg(&adapter_path)
        .arg("--metrics-output")
        .arg(&metrics_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 =
            0x08000000;

        command.creation_flags(
            CREATE_NO_WINDOW
        );
    }

    let mut child =
        LoraTrainerChildGuardV1(
            command
                .spawn()
                .map_err(|_| {
                    "lora_trainer_spawn_failed"
                        .to_string()
                })?
        );

    let stdout =
        child.stdout
            .take()
            .ok_or_else(|| {
                "lora_trainer_stdout_missing"
                    .to_string()
            })?;

    let stderr =
        child.stderr
            .take()
            .ok_or_else(|| {
                "lora_trainer_stderr_missing"
                    .to_string()
            })?;

    let (
        line_tx,
        line_rx
    ) = mpsc::channel::<(
        bool,
        String
    )>();

    let stdout_tx =
        line_tx.clone();

    thread::spawn(
        move || {
            for line in
                BufReader::new(
                    stdout
                )
                .lines()
                .flatten()
            {
                let _ =
                    stdout_tx.send(
                        (
                            true,
                            line
                        )
                    );
            }
        }
    );

    thread::spawn(
        move || {
            for line in
                BufReader::new(
                    stderr
                )
                .lines()
                .flatten()
            {
                let _ =
                    line_tx.send(
                        (
                            false,
                            line
                        )
                    );
            }
        }
    );

    loop {
        while let Ok(
            (
                is_stdout,
                line
            )
        ) = line_rx.try_recv()
        {
            if is_stdout {
                if let Some(progress) =
                    parse_progress_line_v1(
                        &line
                    )
                {
                    on_progress(
                        &progress
                    )?;
                }
            }
        }

        if stop.load(
            Ordering::Acquire
        ) {
            let _ =
                child.kill();

            let _ =
                child.wait();

            return Err(
                "lora_training_cancelled_by_node_shutdown"
                    .into()
            );
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(
                        format!(
                            "lora_trainer_exit_{}",
                            status
                                .code()
                                .unwrap_or(-1)
                        )
                    );
                }

                break;
            }

            Ok(None) => {}

            Err(_) => {
                return Err(
                    "lora_trainer_wait_failed"
                        .into()
                );
            }
        }

        thread::sleep(
            Duration::from_millis(500)
        );
    }

    if !adapter_path.is_file() {
        return Err(
            "lora_adapter_output_missing"
                .into()
        );
    }

    let adapter_sha256 =
        sha256_file_v1(
            &adapter_path
        )?;

    let metrics =
        if metrics_path.is_file() {
            fs::read_to_string(
                &metrics_path
            )
            .ok()
            .and_then(|raw| {
                serde_json::from_str::<Value>(
                    &raw
                )
                .ok()
            })
            .unwrap_or_else(|| {
                serde_json::json!({})
            })
        } else {
            serde_json::json!({})
        };

    Ok(
        LoraTrainingOutputV1 {
            adapter_path,
            adapter_sha256,
            metrics,
        }
    )
}

#[cfg(test)]
mod preserved_vram_threshold_tests_v1 {
    use super::*;

    #[test]
    fn supported_training_vram_thresholds_are_stable() {
        const GIB: u64 =
            1024 * 1024 * 1024;

        assert_eq!(
            minimum_training_vram_bytes_v1(
                "qwen2.5:3b"
            ),
            Some(8 * GIB)
        );

        assert_eq!(
            minimum_training_vram_bytes_v1(
                "qwen2.5:7b"
            ),
            Some(12 * GIB)
        );

        assert_eq!(
            minimum_training_vram_bytes_v1(
                "qwen2.5:14b"
            ),
            Some(24 * GIB)
        );

        assert_eq!(
            minimum_training_vram_bytes_v1(
                "mistral-small:24b"
            ),
            None
        );
    }
}
