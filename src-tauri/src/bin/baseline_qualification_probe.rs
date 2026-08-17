use edgeswarm_unified_node_lib::{
    core::certification_workload::built_in_3b_realworld_v2,
    runtime::llama_cpp::LlamaCppHttpExecutor,
};

fn main() {
    let base_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:18081".into());

    let pack = built_in_3b_realworld_v2()
        .expect("failed to load 3B certification pack");

    let executor = LlamaCppHttpExecutor::new(base_url)
        .expect("failed to initialize llama.cpp executor");

    println!("PACK_ID={}", pack.pack_id);
    println!("WORKLOAD_COUNT={}", pack.workloads.len());
    println!();

    let mut successful = 0usize;
    let mut valid = 0usize;

    for (index, workload) in pack.workloads.iter().enumerate() {
        println!(
            "=== WORKLOAD {}/{}: {} ===",
            index + 1,
            pack.workloads.len(),
            workload.id
        );

        match executor.execute_one(workload) {
            Ok(execution) => {
                if execution.result.success {
                    successful += 1;
                }

                if execution.result.output_valid {
                    valid += 1;
                }

                println!("PROFILE={:?}", execution.result.profile);
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

                if !execution.result.output_valid {
                    let preview: String =
                        execution.output.chars().take(1800).collect();

                    println!("--- FAILED OUTPUT PREVIEW ---");
                    println!("{preview}");
                    println!("--- END PREVIEW ---");
                }
            }
            Err(error) => {
                println!("SUCCESS=false");
                println!("OUTPUT_VALID=false");
                println!("EXECUTION_ERROR={error}");
            }
        }

        println!();
    }

    println!("=== BASELINE QUALIFICATION SUMMARY ===");
    println!("SUCCESSFUL_WORKLOADS={successful}/{}", pack.workloads.len());
    println!("VALID_WORKLOADS={valid}/{}", pack.workloads.len());
    println!(
        "MODEL_QUALIFIED={}",
        valid == pack.workloads.len()
    );
    println!("CONCURRENCY_TESTED=1");
}
