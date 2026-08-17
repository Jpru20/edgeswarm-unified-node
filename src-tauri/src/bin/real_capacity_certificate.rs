use edgeswarm_unified_node_lib::{
    core::{
        capacity::CapacityCertificateV1,
        capacity_policy::CapacityPolicy,
        capacity_store::{load_certificate, save_certificate},
        certificate_match::sha256_file,
        certification_runner::CertificationRunner,
        certification_workload::built_in_3b_realworld_v2,
        model_registry::OUTPUT_LIMIT_POLICY_VERSION,
        NodeState,
    },
    runtime::{
        llama_cpp::LlamaCppHttpExecutor,
        llama_process::{LlamaProcessConfig, ManagedLlamaProcess},
    },
};
use std::{
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const MODEL_ID: &str = "Qwen2.5-3B-Instruct-Q4_K_M";
const MODEL_SHA256: &str = "9c9f56a391a3abbd5b89d0245bf6106081bcc3173119d4229235dd9d23253f94";

fn runtime_version(path: &Path) -> Result<String, String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|e| format!("runtime_version_failed:{e}"))?;

    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    for line in text.lines() {
        if let Some(version) = line.trim().strip_prefix("version: ") {
            return Ok(version.trim().to_string());
        }
    }

    Err("runtime_version_not_found".into())
}

fn run() -> Result<(), String> {
    let model_path = std::env::args()
        .nth(1)
        .ok_or_else(|| "usage: real_capacity_certificate <model> <runtime>".to_string())?;

    let runtime_path = std::env::args()
        .nth(2)
        .ok_or_else(|| "usage: real_capacity_certificate <model> <runtime>".to_string())?;

    let model_path_string = model_path.clone();
    let model_path = Path::new(&model_path);
    let runtime_path = Path::new(&runtime_path);

    if !model_path.is_file() {
        return Err("model_file_missing".into());
    }

    if !runtime_path.is_file() {
        return Err("runtime_file_missing".into());
    }

    let model_sha = sha256_file(model_path)?;

    if model_sha != MODEL_SHA256 {
        return Err(format!(
            "model_sha_mismatch:expected={MODEL_SHA256}:actual={model_sha}"
        ));
    }

    let runtime_version = runtime_version(runtime_path)?;

    let state = NodeState::detect();
    let pack = built_in_3b_realworld_v2()?;

    let mut policy = CapacityPolicy::default();
    policy.maximum_concurrency = 2;

    let runner = CertificationRunner::new(policy);

    let runtime_config = LlamaProcessConfig::for_model(model_path_string)?;
    let managed_runtime = ManagedLlamaProcess::start(&runtime_config)?;
    let runtime_base_url = managed_runtime.base_url().to_string();

    println!("LLAMA_RUNTIME_OWNERSHIP=managed");
    println!("LLAMA_BASE_URL={runtime_base_url}");

    let mut executor = LlamaCppHttpExecutor::new(runtime_base_url)?;

    println!("REAL_CERTIFICATION_STARTED=true");
    println!("PACK_ID={}", pack.pack_id);
    println!("WORKLOAD_COUNT={}", pack.workloads.len());
    println!("MAXIMUM_CONCURRENCY_TESTED=2");
    println!("MODEL_SHA256={model_sha}");
    println!("RUNTIME_VERSION={runtime_version}");
    println!("ACCELERATION={}", state.acceleration.backend);

    let report = runner.run(&pack, &mut executor)?;

    let baseline = report
        .samples
        .iter()
        .find(|sample| sample.concurrency == 1)
        .ok_or_else(|| "baseline_sample_missing".to_string())?;

    let baseline_valid = baseline.task_results.len() == 6
        && baseline.successful_tasks == 6
        && baseline.failed_tasks == 0
        && baseline.valid_outputs == 6
        && baseline.quality_pass_rate >= 1.0
        && !baseline.thermal_throttled
        && baseline.aggregate_tokens_per_second > 0.0
        && baseline.median_task_wall_time_ms > 0
        && baseline
            .task_results
            .iter()
            .all(|result| result.success && result.output_valid);

    println!("BASELINE_VALID={baseline_valid}");
    println!("BASELINE_WALL_MS={}", baseline.wall_time_ms);
    println!(
        "BASELINE_MEDIAN_TASK_MS={}",
        baseline.median_task_wall_time_ms
    );
    println!(
        "BASELINE_AGGREGATE_TPS={:.4}",
        baseline.aggregate_tokens_per_second
    );

    if !baseline_valid {
        return Err("baseline_certification_failed_refusing_certificate".into());
    }

    let certified_sample = report
        .samples
        .iter()
        .find(|sample| sample.concurrency == report.certified_concurrency)
        .ok_or_else(|| "certified_sample_missing".to_string())?;

    let latency_multiplier =
        certified_sample.median_task_wall_time_ms as f64 / baseline.median_task_wall_time_ms as f64;

    let certificate = CapacityCertificateV1 {
        certificate_version: "edgeswarm-capacity-certificate-v1".into(),
        certification_pack_id: report.certification_pack_id.clone(),
        installation_id: state.identity.installation_id.clone(),
        model_id: MODEL_ID.into(),
        model_sha256: model_sha,
        model_capability: "Neural-Inference-3B".into(),
        quantization: "Q4_K_M".into(),
        runtime: "llama.cpp".into(),
        runtime_version,
        acceleration: state.acceleration.backend.clone(),
        benchmark_mode: "no_cache_prompt".into(),
        capacity_policy_version: "realworld-capacity-policy-v1".into(),
        output_limit_policy_version: OUTPUT_LIMIT_POLICY_VERSION.into(),
        tested_concurrency_levels: report.tested_concurrency_levels.clone(),
        rejected_concurrency: report.rejected_concurrency,
        certified_concurrency: report.certified_concurrency,
        burst_concurrency: None,
        baseline_tokens_per_second: baseline.aggregate_tokens_per_second,
        certified_tokens_per_second: certified_sample.aggregate_tokens_per_second,
        latency_multiplier,
        quality_pass_rate: certified_sample.quality_pass_rate,
        app_version: env!("CARGO_PKG_VERSION").into(),
        created_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system_time_before_epoch".to_string())?
            .as_millis(),
        samples: report.samples.clone(),
    };

    let path = save_certificate(&certificate)?;
    let loaded = load_certificate(&path)?;

    println!(
        "TESTED_CONCURRENCY_LEVELS={:?}",
        loaded.tested_concurrency_levels
    );
    println!("REJECTED_CONCURRENCY={:?}", loaded.rejected_concurrency);
    println!("CERTIFIED_CONCURRENCY={}", loaded.certified_concurrency);
    println!("CERTIFIED_TPS={:.4}", loaded.certified_tokens_per_second);
    println!("QUALITY_PASS_RATE={:.2}", loaded.quality_pass_rate);
    println!("CERTIFICATE_PATH={}", path.display());
    println!("CERTIFICATE_WRITTEN=true");
    println!("REAL_CERTIFICATION_PASS=true");

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("REAL_CERTIFICATION_ERROR={error}");
        eprintln!("CERTIFICATE_WRITTEN=false");
        std::process::exit(1);
    }
}
