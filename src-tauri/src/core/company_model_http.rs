use crate::core::{
    auth_client::SupabaseAuthClient,
    company_model::{
        install_company_adapter_v1,
        CompanyModelDeploymentV1,
    },
    lora_training::{
        LoraTrainingJobV1,
        LoraTrainingProgressV1,
        LoraTrainingStatusV1,
    },
    production_task_http::{
        api_base,
        bearer,
        refresh_auth,
        LocalAuth,
    },
};
use reqwest::blocking::Client;
use serde::{
    Deserialize,
    Serialize,
};
use serde_json::Value;
use std::{
    env,
    fs,
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_DATASET_BYTES_V1: u64 =
    2 * 1024 * 1024 * 1024;

const MAX_ADAPTER_BYTES_V1: u64 =
    8 * 1024 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrainingClaimResponseV1 {
    #[serde(default)]
    job: Option<LoraTrainingJobV1>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompanyDeploymentResponseV1 {
    #[serde(default)]
    deployment:
        Option<CompanyModelDeploymentV1>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainingClaimRequestV1<'a> {
    hardware_id: &'a str,
    base_model_id: &'a str,
    trainer_sha256: &'a str,
    training_memory_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingResultV1<'a> {
    pub hardware_id: &'a str,
    pub adapter_sha256: &'a str,
    pub adapter_artifact_uri: &'a str,
    pub metrics: &'a Value,
}

pub fn company_model_node_enabled_v1(
) -> bool {
    env::var(
        "EDGESWARM_COMPANY_MODEL_V1_ENABLED"
    )
    .map(|value| {
        value == "1" ||
            value
                .eq_ignore_ascii_case(
                    "true"
                )
    })
    .unwrap_or(false)
}

fn parse_https_url_v1(
    raw: &str,
) -> Result<
    reqwest::Url,
    String
> {
    let url =
        reqwest::Url::parse(
            raw
        )
        .map_err(|_| {
            "company_model_signed_url_invalid"
                .to_string()
        })?;

    if url.scheme() != "https" ||
        url.host_str().is_none() ||
        !url.username().is_empty() ||
        url.password().is_some()
    {
        return Err(
            "company_model_signed_url_rejected"
                .into()
        );
    }

    Ok(url)
}

pub fn download_signed_file_v1(
    http: &Client,
    raw_url: &str,
    destination: &Path,
    expected_sha256: &str,
    max_bytes: u64,
) -> Result<u64, String> {
    let url =
        parse_https_url_v1(
            raw_url
        )?;

    let mut response =
        http
            .get(url)
            .send()
            .map_err(|_| {
                "company_model_download_failed"
                    .to_string()
            })?;

    if !response
        .status()
        .is_success()
    {
        return Err(
            format!(
                "company_model_download_http_{}",
                response
                    .status()
                    .as_u16()
            )
        );
    }

    if let Some(length) =
        response.content_length()
    {
        if length > max_bytes {
            return Err(
                "company_model_download_too_large"
                    .into()
            );
        }
    }

    if let Some(parent) =
        destination.parent()
    {
        fs::create_dir_all(
            parent
        )
        .map_err(|_| {
            "company_model_download_directory_failed"
                .to_string()
        })?;
    }

    let temp =
        destination.with_extension(
            "download"
        );

    let mut output =
        fs::File::create(
            &temp
        )
        .map_err(|_| {
            "company_model_download_file_create_failed"
                .to_string()
        })?;

    let mut total =
        0u64;

    let mut buffer =
        [0u8; 1024 * 1024];

    loop {
        let read =
            response
                .read(
                    &mut buffer
                )
                .map_err(|_| {
                    "company_model_download_read_failed"
                        .to_string()
                })?;

        if read == 0 {
            break;
        }

        total =
            total
                .saturating_add(
                    read as u64
                );

        if total > max_bytes {
            let _ =
                fs::remove_file(
                    &temp
                );

            return Err(
                "company_model_download_too_large"
                    .into()
            );
        }

        output
            .write_all(
                &buffer[..read]
            )
            .map_err(|_| {
                "company_model_download_write_failed"
                    .to_string()
            })?;
    }

    output
        .sync_all()
        .map_err(|_| {
            "company_model_download_sync_failed"
                .to_string()
        })?;

    let actual_hash =
        crate::core::
            lora_training::
            sha256_file_v1(
                &temp
            )?;

    if !actual_hash
        .eq_ignore_ascii_case(
            expected_sha256
        )
    {
        let _ =
            fs::remove_file(
                &temp
            );

        return Err(
            "company_model_download_hash_mismatch"
                .into()
        );
    }

    fs::rename(
        &temp,
        destination,
    )
    .map_err(|_| {
        "company_model_download_commit_failed"
            .to_string()
    })?;

    Ok(total)
}

pub fn upload_signed_file_v1(
    http: &Client,
    raw_url: &str,
    path: &Path,
) -> Result<u64, String> {
    let url =
        parse_https_url_v1(
            raw_url
        )?;

    let metadata =
        fs::metadata(
            path
        )
        .map_err(|_| {
            "company_model_upload_file_missing"
                .to_string()
        })?;

    if metadata.len() >
        MAX_ADAPTER_BYTES_V1
    {
        return Err(
            "company_model_adapter_too_large"
                .into()
        );
    }

    let file =
        fs::File::open(
            path
        )
        .map_err(|_| {
            "company_model_upload_open_failed"
                .to_string()
        })?;

    let response =
        http
            .put(url)
            .header(
                "content-type",
                "application/octet-stream"
            )
            .body(file)
            .send()
            .map_err(|_| {
                "company_model_upload_failed"
                    .to_string()
            })?;

    if !response
        .status()
        .is_success()
    {
        return Err(
            format!(
                "company_model_upload_http_{}",
                response
                    .status()
                    .as_u16()
            )
        );
    }

    Ok(
        metadata.len()
    )
}

pub fn fetch_company_deployment_v1(
    http: &Client,
    auth_client:
        &SupabaseAuthClient,
    auth:
        &mut LocalAuth,
    hardware_id: &str,
) -> Result<
    Option<CompanyModelDeploymentV1>,
    String
> {
    if !company_model_node_enabled_v1() {
        return Ok(None);
    }

    for auth_try in 0..2 {
        let response =
            http
                .get(
                    format!(
                        "{}/enterprise/hive/company-model/node-state",
                        api_base()
                    )
                )
                .header(
                    "authorization",
                    bearer(
                        &auth
                            .access_token
                    )?
                )
                .query(&[
                    (
                        "hardwareId",
                        hardware_id
                    )
                ])
                .send()
                .map_err(|_| {
                    "company_model_deployment_poll_failed"
                        .to_string()
                })?;

        let status =
            response
                .status()
                .as_u16();

        if status == 401 &&
            auth_try == 0
        {
            let stale =
                auth
                    .access_token
                    .clone();

            *auth =
                refresh_auth(
                    auth_client,
                    &stale
                )?;

            continue;
        }

        if status == 404 ||
            status == 204
        {
            return Ok(None);
        }

        if !(200..300)
            .contains(
                &status
            )
        {
            return Err(
                format!(
                    "company_model_deployment_poll_http_{status}"
                )
            );
        }

        let body =
            response
                .json::<
                    CompanyDeploymentResponseV1
                >()
                .map_err(|_| {
                    "company_model_deployment_response_invalid"
                        .to_string()
                })?;

        return Ok(
            body.deployment
        );
    }

    Err(
        "company_model_deployment_auth_failed"
            .into()
    )
}

pub fn provision_company_deployment_v1(
    http: &Client,
    deployment:
        &CompanyModelDeploymentV1,
) -> Result<(), String> {
    if !deployment
        .status
        .eq_ignore_ascii_case(
            "certified"
        )
    {
        return Err(
            "company_model_deployment_not_certified"
                .into()
        );
    }

    if let Some(current) =
        crate::core::
            company_model::
            load_local_company_model_v1()
    {
        if current
            .company_model_id
            .eq_ignore_ascii_case(
                &deployment
                    .company_model_id
            ) &&
            current.version ==
                deployment.version &&
            current
                .adapter_sha256
                .eq_ignore_ascii_case(
                    &deployment
                        .adapter_sha256
                ) &&
            Path::new(
                &current.adapter_path
            )
            .is_file()
        {
            return Ok(());
        }
    }

    let staging_root =
        crate::core::
            company_model::
            company_model_root_v1()
            .join("staging");

    fs::create_dir_all(
        &staging_root
    )
    .map_err(|_| {
        "company_model_staging_directory_failed"
            .to_string()
    })?;

    let temp_adapter =
        staging_root.join(
            format!(
                "{}-v{}.gguf",
                deployment
                    .company_model_id,
                deployment.version
            )
        );

    download_signed_file_v1(
        http,
        &deployment
            .adapter_download_url,
        &temp_adapter,
        &deployment
            .adapter_sha256,
        MAX_ADAPTER_BYTES_V1,
    )?;

    install_company_adapter_v1(
        deployment,
        &temp_adapter,
    )?;

    let _ =
        fs::remove_file(
            temp_adapter
        );

    Ok(())
}

pub fn claim_training_job_v1(
    http: &Client,
    auth_client:
        &SupabaseAuthClient,
    auth:
        &mut LocalAuth,
    hardware_id: &str,
    training:
        &LoraTrainingStatusV1,
) -> Result<
    Option<LoraTrainingJobV1>,
    String
> {
    if !company_model_node_enabled_v1() ||
        !training.enabled
    {
        return Ok(None);
    }

    let base_model_id =
        training
            .base_model_id
            .as_deref()
            .ok_or_else(|| {
                "training_base_model_missing"
                    .to_string()
            })?;

    let trainer_sha256 =
        training
            .trainer_sha256
            .as_deref()
            .ok_or_else(|| {
                "training_trainer_hash_missing"
                    .to_string()
            })?;

    let training_memory_bytes =
        training
            .training_memory_bytes
            .unwrap_or(0);

    let payload =
        TrainingClaimRequestV1 {
            hardware_id,
            base_model_id,
            trainer_sha256,
            training_memory_bytes,
        };

    for auth_try in 0..2 {
        let response =
            http
                .post(
                    format!(
                        "{}/enterprise/hive/training/jobs/claim",
                        api_base()
                    )
                )
                .header(
                    "authorization",
                    bearer(
                        &auth
                            .access_token
                    )?
                )
                .json(
                    &payload
                )
                .send()
                .map_err(|_| {
                    "training_job_claim_failed"
                        .to_string()
                })?;

        let status =
            response
                .status()
                .as_u16();

        if status == 401 &&
            auth_try == 0
        {
            let stale =
                auth
                    .access_token
                    .clone();

            *auth =
                refresh_auth(
                    auth_client,
                    &stale
                )?;

            continue;
        }

        if status == 404 ||
            status == 204
        {
            return Ok(None);
        }

        if !(200..300)
            .contains(
                &status
            )
        {
            return Err(
                format!(
                    "training_job_claim_http_{status}"
                )
            );
        }

        let body =
            response
                .json::<
                    TrainingClaimResponseV1
                >()
                .map_err(|_| {
                    "training_job_claim_response_invalid"
                        .to_string()
                })?;

        if let Some(job) =
            body.job
                .as_ref()
        {
            if !job
                .base_model_id
                .eq_ignore_ascii_case(
                    base_model_id
                )
            {
                return Err(
                    "training_job_base_model_mismatch"
                        .into()
                );
            }
        }

        return Ok(
            body.job
        );
    }

    Err(
        "training_job_claim_auth_failed"
            .into()
    )
}

fn post_job_json_v1<T: Serialize>(
    http: &Client,
    auth_client:
        &SupabaseAuthClient,
    auth:
        &mut LocalAuth,
    job_id: &str,
    suffix: &str,
    body: &T,
) -> Result<u16, String> {
    for auth_try in 0..2 {
        let response =
            http
                .post(
                    format!(
                        "{}/enterprise/hive/training/jobs/{}/{}",
                        api_base(),
                        job_id,
                        suffix
                    )
                )
                .header(
                    "authorization",
                    bearer(
                        &auth
                            .access_token
                    )?
                )
                .json(body)
                .send()
                .map_err(|_| {
                    "training_job_update_failed"
                        .to_string()
                })?;

        let status =
            response
                .status()
                .as_u16();

        if status == 401 &&
            auth_try == 0
        {
            let stale =
                auth
                    .access_token
                    .clone();

            *auth =
                refresh_auth(
                    auth_client,
                    &stale
                )?;

            continue;
        }

        if !(200..300)
            .contains(
                &status
            )
        {
            return Err(
                format!(
                    "training_job_update_http_{status}"
                )
            );
        }

        return Ok(status);
    }

    Err(
        "training_job_update_auth_failed"
            .into()
    )
}

pub fn report_training_progress_v1(
    http: &Client,
    auth_client:
        &SupabaseAuthClient,
    auth:
        &mut LocalAuth,
    job_id: &str,
    hardware_id: &str,
    progress:
        &LoraTrainingProgressV1,
) -> Result<u16, String> {
    post_job_json_v1(
        http,
        auth_client,
        auth,
        job_id,
        "progress",
        &serde_json::json!({
            "hardwareId":
                hardware_id,
            "status":
                "running",
            "progress":
                progress
        }),
    )
}

pub fn report_training_result_v1(
    http: &Client,
    auth_client:
        &SupabaseAuthClient,
    auth:
        &mut LocalAuth,
    job_id: &str,
    result:
        &TrainingResultV1<'_>,
) -> Result<u16, String> {
    post_job_json_v1(
        http,
        auth_client,
        auth,
        job_id,
        "result",
        result,
    )
}

pub fn report_training_failure_v1(
    http: &Client,
    auth_client:
        &SupabaseAuthClient,
    auth:
        &mut LocalAuth,
    job_id: &str,
    hardware_id: &str,
    reason: &str,
) -> Result<u16, String> {
    post_job_json_v1(
        http,
        auth_client,
        auth,
        job_id,
        "failure",
        &serde_json::json!({
            "hardwareId":
                hardware_id,
            "reason":
                reason
        }),
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrainingCleanupReportV1 {
    pub files_removed: u64,
    pub directories_removed: u64,
    pub bytes_overwrite_attempted: u64,
    pub overwrite_failures: u64,
    pub remove_failures: u64,
}

impl TrainingCleanupReportV1 {
    fn new() -> Self {
        Self {
            files_removed: 0,
            directories_removed: 0,
            bytes_overwrite_attempted: 0,
            overwrite_failures: 0,
            remove_failures: 0,
        }
    }

    pub fn clean(&self) -> bool {
        self.remove_failures == 0
    }
}

fn safe_training_job_id_v1(
    job_id: &str,
) -> Result<String, String> {
    let value =
        job_id.trim();

    if value.is_empty() ||
        value.len() > 128 ||
        !value
            .chars()
            .all(|c| {
                c.is_ascii_alphanumeric() ||
                c == '-' ||
                c == '_'
            })
    {
        return Err(
            "training_job_id_invalid"
                .into()
        );
    }

    Ok(value.to_string())
}

pub fn training_job_root_v1(
    job_id: &str,
) -> Result<PathBuf, String> {
    let safe_id =
        safe_training_job_id_v1(
            job_id
        )?;

    Ok(
        crate::adapters::
            app_data_dir()
            .join("training")
            .join("jobs")
            .join(safe_id)
    )
}

fn best_effort_overwrite_file_v1(
    path: &Path,
    report: &mut TrainingCleanupReportV1,
) {
    let metadata =
        match fs::symlink_metadata(
            path
        ) {
            Ok(value) => value,
            Err(_) => return,
        };

    if !metadata
        .file_type()
        .is_file()
    {
        return;
    }

    let length =
        metadata.len();

    report.bytes_overwrite_attempted =
        report
            .bytes_overwrite_attempted
            .saturating_add(length);

    if length == 0 {
        return;
    }

    let mut file =
        match OpenOptions::new()
            .write(true)
            .open(path)
        {
            Ok(value) => value,
            Err(_) => {
                report.overwrite_failures += 1;
                return;
            }
        };

    let zeros =
        vec![0u8; 1024 * 1024];

    let mut remaining =
        length;

    while remaining > 0 {
        let write_len =
            usize::try_from(
                remaining.min(
                    zeros.len() as u64
                )
            )
            .unwrap_or(
                zeros.len()
            );

        if file
            .write_all(
                &zeros[..write_len]
            )
            .is_err()
        {
            report.overwrite_failures += 1;
            return;
        }

        remaining -=
            write_len as u64;
    }

    if file.sync_all().is_err() {
        report.overwrite_failures += 1;
    }
}

fn cleanup_path_v1(
    path: &Path,
    report: &mut TrainingCleanupReportV1,
) {
    let metadata =
        match fs::symlink_metadata(
            path
        ) {
            Ok(value) => value,
            Err(_) => return,
        };

    let file_type =
        metadata.file_type();

    // Never follow symlinks / junction-like links during cleanup.
    if file_type.is_symlink() {
        if fs::remove_file(path).is_ok() {
            report.files_removed += 1;
        } else if fs::remove_dir(path).is_ok() {
            report.directories_removed += 1;
        } else {
            report.remove_failures += 1;
        }

        return;
    }

    if file_type.is_file() {
        best_effort_overwrite_file_v1(
            path,
            report,
        );

        if fs::remove_file(path).is_ok() {
            report.files_removed += 1;
        } else {
            report.remove_failures += 1;
        }

        return;
    }

    if file_type.is_dir() {
        if let Ok(entries) =
            fs::read_dir(path)
        {
            for entry in
                entries.flatten()
            {
                cleanup_path_v1(
                    &entry.path(),
                    report,
                );
            }
        }

        if fs::remove_dir(path).is_ok() {
            report.directories_removed += 1;
        } else if path.exists() {
            report.remove_failures += 1;
        }
    }
}

pub fn cleanup_training_job_workspace_v1(
    job_id: &str,
) -> Result<TrainingCleanupReportV1, String> {
    let root =
        training_job_root_v1(
            job_id
        )?;

    let mut report =
        TrainingCleanupReportV1::new();

    cleanup_path_v1(
        &root,
        &mut report,
    );

    Ok(report)
}

pub fn cleanup_all_training_workspaces_v1(
) -> TrainingCleanupReportV1 {
    let jobs_root =
        crate::adapters::
            app_data_dir()
            .join("training")
            .join("jobs");

    let mut report =
        TrainingCleanupReportV1::new();

    if let Ok(entries) =
        fs::read_dir(
            &jobs_root
        )
    {
        for entry in
            entries.flatten()
        {
            cleanup_path_v1(
                &entry.path(),
                &mut report,
            );
        }
    }

    // Keep the jobs parent available for the next claim.
    let _ =
        fs::create_dir_all(
            &jobs_root
        );

    report
}

pub struct TrainingWorkspaceGuardV1 {
    job_id: String,
    armed: bool,
}

impl TrainingWorkspaceGuardV1 {
    pub fn new(
        job_id: &str,
    ) -> Result<Self, String> {
        let safe_id =
            safe_training_job_id_v1(
                job_id
            )?;

        let root =
            training_job_root_v1(
                &safe_id
            )?;

        fs::create_dir_all(
            &root
        )
        .map_err(|_| {
            "training_job_directory_failed"
                .to_string()
        })?;

        Ok(Self {
            job_id: safe_id,
            armed: true,
        })
    }

    pub fn cleanup_now(
        &mut self,
    ) -> Result<TrainingCleanupReportV1, String> {
        let report =
            cleanup_training_job_workspace_v1(
                &self.job_id
            )?;

        // If a Windows handle still has a file open, keep the guard
        // armed so Drop gets one more best-effort cleanup attempt.
        if report.clean() {
            self.armed = false;
        }

        Ok(report)
    }
}

impl Drop for TrainingWorkspaceGuardV1 {
    fn drop(&mut self) {
        if self.armed {
            let _ =
                cleanup_training_job_workspace_v1(
                    &self.job_id
                );

            self.armed = false;
        }
    }
}

pub fn download_training_dataset_v1(
    http: &Client,
    job:
        &LoraTrainingJobV1,
) -> Result<PathBuf, String> {
    let root =
        training_job_root_v1(
            &job.id
        )?;

    fs::create_dir_all(
        &root
    )
    .map_err(|_| {
        "training_job_directory_failed"
            .to_string()
    })?;

    let dataset =
        root.join(
            "dataset.jsonl"
        );

    download_signed_file_v1(
        http,
        &job
            .dataset_download_url,
        &dataset,
        &job
            .dataset_sha256,
        MAX_DATASET_BYTES_V1,
    )?;

    Ok(dataset)
}

pub fn training_output_dir_v1(
    job_id: &str,
) -> Result<PathBuf, String> {
    let root =
        training_job_root_v1(
            job_id
        )?
        .join("output");

    fs::create_dir_all(
        &root
    )
    .map_err(|_| {
        "training_output_directory_failed"
            .to_string()
    })?;

    Ok(root)
}
