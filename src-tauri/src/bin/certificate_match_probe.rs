use edgeswarm_unified_node_lib::core::{
    capacity_store::load_certificate,
    certificate_match::{
        certificate_match_failures,
        sha256_file,
        CertificateMatchContext,
    },
    model_registry::OUTPUT_LIMIT_POLICY_VERSION,
    NodeState,
};
use std::path::PathBuf;

fn main() {
    let certificate_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: certificate_match_probe <certificate> <model> <runtime-version>")
    );

    let model_path = PathBuf::from(
        std::env::args()
            .nth(2)
            .expect("model path missing")
    );

    let runtime_version = std::env::args()
        .nth(3)
        .expect("runtime version missing");

    let state = NodeState::detect();

    let certificate = load_certificate(&certificate_path)
        .expect("failed to load capacity certificate");

    let actual_sha = sha256_file(&model_path)
        .expect("failed to hash installed model");

    println!("CERTIFICATE_PATH={}", certificate_path.display());
    println!("MODEL_PATH={}", model_path.display());
    println!("MODEL_SHA256_ACTUAL={actual_sha}");
    println!(
        "MODEL_SHA256_CERTIFIED={}",
        certificate.model_sha256
    );
    println!(
        "ACCELERATION_ACTUAL={}",
        state.acceleration.backend
    );
    println!("RUNTIME_VERSION_ACTUAL={runtime_version}");

    let context = CertificateMatchContext {
        installation_id: &state.identity.installation_id,
        model_id: "Qwen2.5-3B-Instruct-Q4_K_M",
        model_sha256: &actual_sha,
        model_capability: "Neural-Inference-3B",
        quantization: "Q4_K_M",
        runtime: "llama.cpp",
        runtime_version: &runtime_version,
        acceleration: &state.acceleration.backend,
        certification_pack_id: "edgeswarm-3b-realworld-v2",
        benchmark_mode: "no_cache_prompt",
        capacity_policy_version: "realworld-capacity-policy-v1",
        output_limit_policy_version: OUTPUT_LIMIT_POLICY_VERSION,
    };

    let failures =
        certificate_match_failures(&certificate, &context);

    println!(
        "CERTIFIED_CONCURRENCY={}",
        certificate.certified_concurrency
    );

    println!(
        "POLICY_RECOMPUTE_SUPPORTED=true"
    );

    if failures.is_empty() {
        println!("CERTIFICATE_MATCH=true");
        println!("CERTIFICATE_STATUS=certified");
    } else {
        println!("CERTIFICATE_MATCH=false");

        for failure in failures {
            println!("MATCH_FAILURE={failure}");
        }

        println!("CERTIFICATE_STATUS=revalidation_required");
    }
}
