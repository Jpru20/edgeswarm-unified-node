use crate::{
    core::{
        capacity::CapacityCertificateV1,
        capacity_policy::CapacityPolicy,
        capacity_store::{load_certificate, save_certificate},
        certificate_match::sha256_file,
        certification_runner::CertificationRunner,
        certification_workload::{bind_neural_realworld_pack_v1, built_in_neural_realworld_v1},
        model_discovery::discover_models,
        model_registry::{model_spec, OUTPUT_LIMIT_POLICY_VERSION},
        NodeState,
    },
    runtime::{
        llama_cpp::LlamaCppHttpExecutor,
        llama_process::{start_managed_llama_with_cpu_fallback_v1, LlamaProcessConfig},
    },
};
use reqwest::blocking::Client;
use serde_json::Value;
use std::{
    path::Path,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn approved_model_from_sha(model_sha: &str) -> Option<(&'static str, &'static str)> {
    match model_sha {
        "9c9f56a391a3abbd5b89d0245bf6106081bcc3173119d4229235dd9d23253f94" => {
            Some(("Qwen2.5-3B-Instruct-Q4_K_M", "Neural-Inference-3B"))
        }
        "65b8fcd92af6b4fefa935c625d1ac27ea29dcb6ee14589c55a8f115ceaaa1423" => {
            Some(("Qwen2.5-7B-Instruct-Q4_K_M", "Neural-Inference-7B"))
        }
        "7b064f5842bf9532c91456deda288a1b672397a54fa729aa665952863033557c" => {
            Some(("Meta-Llama-3.1-8B-Instruct-Q4_K_M", "Neural-Inference-8B"))
        }
        "e47ad95dad6ff848b431053b375adb5d39321290ea2c638682577dafca87c008" => {
            Some(("Qwen2.5-14B-Instruct-Q4_K_M", "Neural-Inference-14B"))
        }
        "2946d28c9e1bb2bcae6d42e8678863a31775df6f740315c7d7e6d6b6411f5937" => {
            Some(("Qwen2.5-Coder-14B-Instruct-Q4_K_M", "Neural-Inference-14B"))
        }
        "d1a6d049f09730c3f8ba26cf6b0b60c89790b5fdafa9a59c819acdfe93fffd1b" => Some((
            "Mistral-Small-24B-Instruct-2501-Q4_K_M",
            "Neural-Inference-24B",
        )),
        "4e83142e3ad3719ac61334f70a956dcc60bbba8adb29de5114161310bb9f7170" => {
            Some(("Gemma-3-27B-it-Q4_K_M", "Neural-Inference-27B"))
        }
        "382b4f5a164d200f93790ee0e339fae12852896d23485cfb203ce868fea33a95" => {
            Some(("Qwen3-30B-A3B-Instruct-Q4_K_M", "Neural-Inference-30B"))
        }
        "4cc57c0f51040a226e5a72cc47b7613f7772950e460a665f7083de89f183f60e" => {
            Some(("Muse-Glimmer-30B-Q4_K_M", "Neural-Inference-30B"))
        }
        "f775c87029be95fb41df9e2882e6e938b73121c30ffc235ac6b6b880add49aa5" => Some((
            "Meta-Llama-3.1-70B-Instruct-Q4_K_M",
            "Neural-Inference-70B-Plus",
        )),
        _ => None,
    }
}

pub fn reported_runtime_slot_count_v1(
    base_url: &str,
) -> Result<u16, String> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .no_proxy()
        .build()
        .map_err(|_| "llama_slot_client_failed".to_string())?;

    let response = client
        .get(format!(
            "{}/slots",
            base_url.trim_end_matches('/')
        ))
        .send()
        .map_err(|_| "llama_slots_request_failed".to_string())?;

    if !response.status().is_success() {
        return Err(format!(
            "llama_slots_http_{}",
            response.status().as_u16()
        ));
    }

    let slots = response
        .json::<Vec<Value>>()
        .map_err(|_| "llama_slots_response_invalid".to_string())?;

    let count =
        u16::try_from(slots.len())
            .map_err(|_| "llama_slot_count_overflow".to_string())?;

    if count == 0 {
        return Err("llama_slot_count_zero".into());
    }

    Ok(count)
}

pub fn certification_max_concurrency_v1(
    reported_slots: u16,
) -> u16 {
    CapacityPolicy::default()
        .maximum_concurrency
        .min(reported_slots.max(1))
}

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
        if let Some((_, version)) = line.trim().split_once("version: ") {
            return Ok(version.trim().to_string());
        }
    }

    Err("runtime_version_not_found".into())
}

pub fn certify_model_path_v1(
    model_path_string: String,
    runtime_path_string: String,
) -> Result<(), String> {
    let model_path = Path::new(&model_path_string);
    let runtime_path = Path::new(&runtime_path_string);

    if !model_path.is_file() {
        return Err("model_file_missing".into());
    }

    if !runtime_path.is_file() {
        return Err("runtime_file_missing".into());
    }

    let model_sha = sha256_file(model_path)?;

    let (model_id, model_capability) = approved_model_from_sha(&model_sha)
        .ok_or_else(|| format!("model_not_approved_for_real_certification:{model_sha}"))?;

    let model_parent = model_path.parent()
        .ok_or_else(|| "certification_model_parent_missing".to_string())?;
    let canonical_model = std::fs::canonicalize(model_path)
        .map_err(|_| "certification_model_canonicalize_failed".to_string())?;

    let selected_model = discover_models(model_parent)
        .into_iter()
        .find(|candidate| {
            std::fs::canonicalize(&candidate.path)
                .map(|path| path == canonical_model)
                .unwrap_or(false)
        })
        .map(|candidate| candidate.selected_model)
        .ok_or_else(|| "certification_selected_model_not_resolved".to_string())?;

    let selected_spec = model_spec(&selected_model)
        .ok_or_else(|| format!("model_spec_missing:{selected_model}"))?;

    if selected_spec.capability != model_capability {
        return Err("certification_selected_model_capability_mismatch".into());
    }


    let state = NodeState::detect();
    let mut pack = built_in_neural_realworld_v1()?;
    bind_neural_realworld_pack_v1(&mut pack, model_capability)?;

    let mut runtime_config = LlamaProcessConfig::for_model(model_path_string)?;
    runtime_config.executable = runtime_path.to_path_buf();

    println!("MODEL_ID={model_id}");
    println!("MODEL_CAPABILITY={model_capability}");
    println!("GPU_LAYERS={}", runtime_config.gpu_layers);

    let managed_runtime =
        start_managed_llama_with_cpu_fallback_v1(&runtime_config)?;
    let runtime_version =
        runtime_version(managed_runtime.executable_path())?;
    let runtime_acceleration =
        managed_runtime.acceleration().to_string();
    let runtime_base_url = managed_runtime.base_url().to_string();

    let reported_slots =
        reported_runtime_slot_count_v1(
            &runtime_base_url
        )
        .unwrap_or_else(|error| {
            println!(
                "LLAMA_SLOT_DISCOVERY_FAILED={error}"
            );
            1
        });

    let maximum_concurrency =
        certification_max_concurrency_v1(
            reported_slots
        );

    let mut policy =
        CapacityPolicy::default();

    policy.maximum_concurrency =
        maximum_concurrency;

    let runner =
        CertificationRunner::new(policy);

    println!(
        "LLAMA_REPORTED_SLOT_COUNT={reported_slots}"
    );

    println!(
        "CERTIFICATION_DYNAMIC_MAX_CONCURRENCY={maximum_concurrency}"
    );

    println!("LLAMA_RUNTIME_OWNERSHIP=managed");
    println!("LLAMA_BASE_URL={runtime_base_url}");

    println!("SELECTED_MODEL={selected_model}");
    let mut executor = LlamaCppHttpExecutor::new_for_model(runtime_base_url, &selected_model)?;

    println!("REAL_CERTIFICATION_STARTED=true");
    println!("PACK_ID={}", pack.pack_id);
    println!("WORKLOAD_COUNT={}", pack.workloads.len());
    println!("MAXIMUM_CONCURRENCY_TESTED={maximum_concurrency}");
    println!("MODEL_SHA256={model_sha}");
    println!("RUNTIME_VERSION={runtime_version}");
    println!("ACCELERATION={runtime_acceleration}");

    crate::core::certification_progress::certification_begin_v1(
        maximum_concurrency,
        pack.workloads.len(),
    );

    let report =
        match runner.run(&pack, &mut executor) {
            Ok(report) => report,
            Err(error) => {
                crate::core::certification_progress::certification_error_v1(
                    error.clone()
                );
                return Err(error);
            }
        };

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
        model_id: model_id.into(),
        model_sha256: model_sha,
        model_capability: model_capability.into(),
        quantization: "Q4_K_M".into(),
        runtime: "llama.cpp".into(),
        runtime_version,
        acceleration: runtime_acceleration,
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

    crate::core::certification_progress::certification_complete_v1(
        loaded.certified_concurrency,
        loaded.rejected_concurrency,
        loaded.tested_concurrency_levels.clone(),
    );

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
