use crate::core::{
    capacity::CapacityCertificateV1,
    capacity_policy::CapacityPolicy,
};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as FmtWrite,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

pub struct CertificateMatchContext<'a> {
    pub installation_id: &'a str,
    pub model_id: &'a str,
    pub model_sha256: &'a str,
    pub model_capability: &'a str,
    pub quantization: &'a str,
    pub runtime: &'a str,
    pub runtime_version: &'a str,
    pub acceleration: &'a str,
    pub certification_pack_id: &'a str,
    pub benchmark_mode: &'a str,
    pub capacity_policy_version: &'a str,
    pub output_limit_policy_version: &'a str,
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let file = File::open(path)
        .map_err(|e| format!("model_hash_open_failed:{e}"))?;

    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];

    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|e| format!("model_hash_read_failed:{e}"))?;

        if count == 0 {
            break;
        }

        hasher.update(&buffer[..count]);
    }

    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);

    for byte in digest {
        write!(&mut hex, "{byte:02x}")
            .map_err(|e| format!("model_hash_format_failed:{e}"))?;
    }

    Ok(hex)
}

pub fn certificate_match_failures(
    certificate: &CapacityCertificateV1,
    context: &CertificateMatchContext<'_>,
) -> Vec<String> {
    let mut failures = Vec::new();

    macro_rules! check {
        ($field:expr, $expected:expr, $name:expr) => {
            if $field != $expected {
                failures.push(format!(
                    "{}:expected={}:actual={}",
                    $name,
                    $expected,
                    $field
                ));
            }
        };
    }

    check!(
        certificate.installation_id.as_str(),
        context.installation_id,
        "installation_id_mismatch"
    );

    check!(
        certificate.model_id.as_str(),
        context.model_id,
        "model_id_mismatch"
    );

    check!(
        certificate.model_sha256.as_str(),
        context.model_sha256,
        "model_sha256_mismatch"
    );

    check!(
        certificate.model_capability.as_str(),
        context.model_capability,
        "model_capability_mismatch"
    );

    check!(
        certificate.quantization.as_str(),
        context.quantization,
        "quantization_mismatch"
    );

    check!(
        certificate.runtime.as_str(),
        context.runtime,
        "runtime_mismatch"
    );

    check!(
        certificate.runtime_version.as_str(),
        context.runtime_version,
        "runtime_version_mismatch"
    );

    check!(
        certificate.acceleration.as_str(),
        context.acceleration,
        "acceleration_mismatch"
    );

    check!(
        certificate.certification_pack_id.as_str(),
        context.certification_pack_id,
        "certification_pack_mismatch"
    );

    check!(
        certificate.benchmark_mode.as_str(),
        context.benchmark_mode,
        "benchmark_mode_mismatch"
    );

    check!(
        certificate.capacity_policy_version.as_str(),
        context.capacity_policy_version,
        "capacity_policy_version_mismatch"
    );

    check!(
        certificate.output_limit_policy_version.as_str(),
        context.output_limit_policy_version,
        "output_limit_policy_version_mismatch"
    );

    if certificate.certified_concurrency == 0 {
        failures.push("certified_concurrency_zero".into());
    }

    if !certificate
        .tested_concurrency_levels
        .contains(&certificate.certified_concurrency)
    {
        failures.push("certified_level_not_tested".into());
    }

    if certificate.samples.is_empty() {
        failures.push("certificate_samples_missing".into());
    } else {
        let recomputed =
            CapacityPolicy::default().recommend(&certificate.samples);

        if recomputed != certificate.certified_concurrency {
            failures.push(format!(
                "capacity_policy_recompute_mismatch:stored={}:recomputed={}",
                certificate.certified_concurrency,
                recomputed
            ));
        }
    }

    failures
}
