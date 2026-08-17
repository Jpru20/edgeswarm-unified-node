use edgeswarm_unified_node_lib::{
    core::certification_workload::built_in_3b_realworld_v1,
    runtime::llama_cpp::LlamaCppHttpExecutor,
};

fn main() {
    let base_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:18081".into());

    let pack = built_in_3b_realworld_v1()
        .expect("failed to load 3B real-world certification pack");

    let workload = pack
        .workloads
        .iter()
        .find(|w| w.id == "support-payment-reconciliation-01")
        .expect("reference certification workload missing");

    let executor = LlamaCppHttpExecutor::new(base_url)
        .expect("failed to initialize llama.cpp executor");

    let execution = executor
        .execute_one(workload)
        .expect("real certification inference failed");

    println!("WORKLOAD_ID={}", execution.result.workload_id);
    println!("SUCCESS={}", execution.result.success);
    println!("OUTPUT_VALID={}", execution.result.output_valid);
    println!("WALL_TIME_MS={}", execution.result.wall_time_ms);
    println!("OUTPUT_TOKENS={}", execution.result.output_tokens);
    println!(
        "TOKENS_PER_SECOND={:.2}",
        execution.result.tokens_per_second
    );

    if execution.validation_failures.is_empty() {
        println!("VALIDATION_FAILURES=none");
    } else {
        println!(
            "VALIDATION_FAILURES={}",
            execution.validation_failures.join("|")
        );
    }

    println!();
    println!("=== MODEL OUTPUT ===");
    println!("{}", execution.output);
}
