use crate::{
    adapters,
    core::capacity::CapacityCertificateV1,
};
use std::{fs, path::PathBuf};
use uuid::Uuid;

fn safe_fragment(value: &str, maximum: usize) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(maximum)
        .collect()
}

pub fn certificate_path(
    certificate: &CapacityCertificateV1,
) -> Result<PathBuf, String> {
    let identity_path = adapters::identity_file_path();

    let base = identity_path
        .parent()
        .ok_or_else(|| "identity_parent_missing".to_string())?
        .join("capacity_certificates");

    let model_hash =
        safe_fragment(&certificate.model_sha256, 16);

    let runtime =
        safe_fragment(&certificate.runtime_version, 24);

    if model_hash.is_empty() || runtime.is_empty() {
        return Err("certificate_filename_identity_invalid".into());
    }

    Ok(base.join(format!(
        "{}-{}-{}-{}.json",
        safe_fragment(&certificate.model_id, 32),
        model_hash,
        safe_fragment(&certificate.runtime, 16),
        runtime
    )))
}

pub fn save_certificate(
    certificate: &CapacityCertificateV1,
) -> Result<PathBuf, String> {
    if certificate.certified_concurrency == 0 {
        return Err("certified_concurrency_zero".into());
    }

    if !certificate
        .tested_concurrency_levels
        .contains(&certificate.certified_concurrency)
    {
        return Err(
            "certified_level_not_present_in_tested_levels".into()
        );
    }

    if certificate.samples.is_empty() {
        return Err("certificate_samples_missing".into());
    }

    let path = certificate_path(certificate)?;

    let parent = path
        .parent()
        .ok_or_else(|| "certificate_parent_missing".to_string())?;

    fs::create_dir_all(parent)
        .map_err(|e| format!("certificate_directory_failed:{e}"))?;

    let raw = serde_json::to_string_pretty(certificate)
        .map_err(|e| format!("certificate_serialize_failed:{e}"))?;

    let temporary = parent.join(format!(
        ".capacity-certificate-{}.tmp",
        Uuid::new_v4()
    ));

    fs::write(&temporary, raw)
        .map_err(|e| format!("certificate_write_failed:{e}"))?;

    fs::rename(&temporary, &path)
        .map_err(|e| format!("certificate_commit_failed:{e}"))?;

    Ok(path)
}

pub fn load_certificate(
    path: &std::path::Path,
) -> Result<CapacityCertificateV1, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("certificate_read_failed:{e}"))?;

    serde_json::from_str(&raw)
        .map_err(|e| format!("certificate_parse_failed:{e}"))
}
