use edgeswarm_unified_node_lib::core::{
    capacity::{
        CapacityBenchmarkSample,
        CapacityCertificateV1,
        CapacityTaskResult,
    },
    capacity_store::{
        load_certificate,
        save_certificate,
    },
    certification_workload::CertificationProfile,
    model_registry::OUTPUT_LIMIT_POLICY_VERSION,
    NodeState,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn task(
    id: &str,
    profile: CertificationProfile,
    wall_time_ms: u64,
    output_tokens: u32,
    tokens_per_second: f64,
) -> CapacityTaskResult {
    CapacityTaskResult {
        workload_id: id.into(),
        profile,
        success: true,
        output_valid: true,
        wall_time_ms,
        first_token_ms: None,
        output_tokens,
        tokens_per_second,
    }
}

fn sample(
    concurrency: u16,
    wall_time_ms: u64,
    median_task_wall_time_ms: u64,
    task_results: Vec<CapacityTaskResult>,
) -> CapacityBenchmarkSample {
    let total_tokens: u64 = task_results
        .iter()
        .map(|r| r.output_tokens as u64)
        .sum();

    let aggregate_tokens_per_second =
        total_tokens as f64 /
        (wall_time_ms as f64 / 1000.0);

    CapacityBenchmarkSample {
        certification_pack_id:
            "edgeswarm-3b-realworld-v2".into(),
        concurrency,
        wall_time_ms,
        median_task_wall_time_ms,
        median_first_token_ms: None,
        aggregate_tokens_per_second,
        peak_memory_bytes: None,
        thermal_throttled: false,
        successful_tasks: 6,
        failed_tasks: 0,
        valid_outputs: 6,
        quality_pass_rate: 1.0,
        task_results,
    }
}

fn main() {
    let runtime_version = std::env::args()
        .nth(1)
        .expect(
            "usage: persist_capacity_certificate <runtime-version>"
        );

    let state = NodeState::detect();

    let c1 = sample(
        1,
        83_515,
        14_336,
        vec![
            task("sentiment-mixed-01", CertificationProfile::Normal, 14_626, 38, 11.57),
            task("sentiment-negative-02", CertificationProfile::Normal, 14_335, 37, 11.63),
            task("support-triage-billing-01", CertificationProfile::Heavy, 15_067, 43, 11.49),
            task("support-triage-technical-02", CertificationProfile::Heavy, 14_337, 37, 11.62),
            task("email-rewrite-schedule-01", CertificationProfile::Structured, 12_771, 45, 11.82),
            task("email-rewrite-client-02", CertificationProfile::Structured, 12_372, 42, 11.82),
        ],
    );

    let c2 = sample(
        2,
        78_838,
        27_189,
        vec![
            task("sentiment-mixed-01", CertificationProfile::Normal, 27_234, 38, 7.53),
            task("sentiment-negative-02", CertificationProfile::Normal, 27_145, 37, 7.47),
            task("support-triage-billing-01", CertificationProfile::Heavy, 27_449, 38, 7.44),
            task("support-triage-technical-02", CertificationProfile::Heavy, 27_713, 41, 7.63),
            task("email-rewrite-schedule-01", CertificationProfile::Structured, 23_889, 45, 7.18),
            task("email-rewrite-client-02", CertificationProfile::Structured, 23_474, 41, 7.00),
        ],
    );

    let baseline_tps = c1.aggregate_tokens_per_second;

    let certificate = CapacityCertificateV1 {
        certificate_version:
            "edgeswarm-capacity-certificate-v1".into(),
        certification_pack_id:
            "edgeswarm-3b-realworld-v2".into(),
        installation_id:
            state.identity.installation_id.clone(),
        model_id:
            "Qwen2.5-3B-Instruct-Q4_K_M".into(),
        model_sha256:
            "9c9f56a391a3abbd5b89d0245bf6106081bcc3173119d4229235dd9d23253f94".into(),
        model_capability:
            "Neural-Inference-3B".into(),
        quantization:
            "Q4_K_M".into(),
        runtime:
            "llama.cpp".into(),
        runtime_version,
        acceleration:
            "cpu".into(),
        benchmark_mode:
            "no_cache_prompt".into(),
        capacity_policy_version:
            "realworld-capacity-policy-v1".into(),
        output_limit_policy_version:
            OUTPUT_LIMIT_POLICY_VERSION.into(),
        tested_concurrency_levels:
            vec![1, 2],
        rejected_concurrency:
            Some(2),
        certified_concurrency:
            1,
        burst_concurrency:
            None,
        baseline_tokens_per_second:
            baseline_tps,
        certified_tokens_per_second:
            baseline_tps,
        latency_multiplier:
            1.0,
        quality_pass_rate:
            1.0,
        app_version:
            env!("CARGO_PKG_VERSION").into(),
        created_at_unix_ms:
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        samples:
            vec![c1, c2],
    };

    let path = save_certificate(&certificate)
        .expect("failed to persist capacity certificate");

    let loaded = load_certificate(&path)
        .expect("failed to reload capacity certificate");

    assert_eq!(
        loaded.installation_id,
        state.identity.installation_id
    );

    assert_eq!(
        loaded.model_sha256,
        certificate.model_sha256
    );

    println!("CERTIFICATE_PATH={}", path.display());
    println!("INSTALLATION_ID={}", loaded.installation_id);
    println!("MODEL_ID={}", loaded.model_id);
    println!("MODEL_SHA256={}", loaded.model_sha256);
    println!("RUNTIME={}", loaded.runtime);
    println!("RUNTIME_VERSION={}", loaded.runtime_version);
    println!(
        "TESTED_CONCURRENCY_LEVELS={:?}",
        loaded.tested_concurrency_levels
    );
    println!(
        "REJECTED_CONCURRENCY={:?}",
        loaded.rejected_concurrency
    );
    println!(
        "CERTIFIED_CONCURRENCY={}",
        loaded.certified_concurrency
    );
    println!(
        "BASELINE_TPS={:.4}",
        loaded.baseline_tokens_per_second
    );
    println!(
        "QUALITY_PASS_RATE={:.2}",
        loaded.quality_pass_rate
    );
    println!("CERTIFICATE_RELOAD_VALID=true");
}
