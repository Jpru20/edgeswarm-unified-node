use edgeswarm_unified_node_lib::{
    core::certification_workload::built_in_3b_realworld_v2,
    runtime::llama_cpp::LlamaCppHttpExecutor,
};

fn main() {
    let base_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:18086".into());

    let mut pack =
        built_in_3b_realworld_v2().expect("failed to load 3B v2 pack");

    let mut workload = pack.workloads.remove(0);

    workload.max_output_tokens = 16;

    println!("WORKLOAD_ID={}", workload.id);
    println!("INITIAL_MAX_OUTPUT_TOKENS={}", workload.max_output_tokens);

    let executor =
        LlamaCppHttpExecutor::new(base_url).expect("executor init failed");

    let execution =
        executor.execute_one(&workload).expect("inference failed");

    println!("ATTEMPTS={}", execution.attempts);
    println!("RETRY_TRIGGERED={}", execution.attempts > 1);
    println!("FINAL_MAX_TOKENS_USED={}", execution.max_tokens_used);
    println!("FINAL_FINISH_REASON={:?}", execution.finish_reason);
    println!("FINAL_TRUNCATED={}", execution.truncated);
    println!("SUCCESS={}", execution.result.success);
    println!("OUTPUT_VALID={}", execution.result.output_valid);
    println!("OUTPUT_TOKENS={}", execution.result.output_tokens);

    if execution.validation_failures.is_empty() {
        println!("VALIDATION_FAILURES=none");
    } else {
        println!(
            "VALIDATION_FAILURES={:?}",
            execution.validation_failures
        );
    }

    println!("OUTPUT={}", execution.output);

    assert_eq!(
        execution.attempts, 2,
        "first constrained attempt did not trigger retry"
    );

    assert!(
        execution.max_tokens_used > workload.max_output_tokens,
        "retry budget did not expand"
    );

    assert!(
        !execution.truncated,
        "final answer remained truncated"
    );

    assert!(
        execution.result.success,
        "final answer was not successful"
    );

    assert!(
        execution.result.output_valid,
        "final answer failed validation"
    );

    println!("COMPLETION_AWARE_RETRY_VALID=true");
}
