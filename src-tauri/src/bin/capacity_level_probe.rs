use edgeswarm_unified_node_lib::{
    core::{
        certification_runner::CertificationExecutor,
        certification_workload::built_in_3b_realworld_v2,
    },
    runtime::llama_cpp::LlamaCppHttpExecutor,
};

fn median(mut values: Vec<u64>) -> u64 {
    values.sort_unstable();
    let n = values.len();

    if n == 0 {
        return 0;
    }

    if n % 2 == 0 {
        (values[n / 2 - 1] + values[n / 2]) / 2
    } else {
        values[n / 2]
    }
}

fn main() {
    let concurrency: u16 = std::env::args()
        .nth(1)
        .expect("usage: capacity_level_probe <concurrency> [base-url]")
        .parse()
        .expect("invalid concurrency");

    let base_url = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "http://127.0.0.1:18083".into());

    let pack = built_in_3b_realworld_v2().unwrap();

    let mut executor =
        LlamaCppHttpExecutor::new(base_url).unwrap();

    let batch = executor
        .execute(&pack.workloads, concurrency)
        .unwrap();

    let successful = batch
        .task_results
        .iter()
        .filter(|r| r.success)
        .count();

    let valid = batch
        .task_results
        .iter()
        .filter(|r| r.output_valid)
        .count();

    let total_tokens: u64 = batch
        .task_results
        .iter()
        .map(|r| r.output_tokens as u64)
        .sum();

    let aggregate_tps = if batch.wall_time_ms > 0 {
        total_tokens as f64 / (batch.wall_time_ms as f64 / 1000.0)
    } else {
        0.0
    };

    let median_wall = median(
        batch
            .task_results
            .iter()
            .map(|r| r.wall_time_ms)
            .collect(),
    );

    println!("PACK_ID={}", pack.pack_id);
    println!("CONCURRENCY={concurrency}");

    for result in &batch.task_results {
        println!(
            "{} | valid={} | wall_ms={} | tokens={} | runtime_tps={:.2}",
            result.workload_id,
            result.output_valid,
            result.wall_time_ms,
            result.output_tokens,
            result.tokens_per_second
        );
    }

    println!();
    println!("=== CAPACITY LEVEL SUMMARY ===");
    println!("SUCCESSFUL_WORKLOADS={successful}/{}", pack.workloads.len());
    println!("VALID_WORKLOADS={valid}/{}", pack.workloads.len());
    println!("PACK_WALL_TIME_MS={}", batch.wall_time_ms);
    println!("MEDIAN_TASK_WALL_TIME_MS={median_wall}");
    println!("TOTAL_OUTPUT_TOKENS={total_tokens}");
    println!("AGGREGATE_TOKENS_PER_SECOND={aggregate_tps:.2}");
    println!("QUALITY_PASS={}", valid == pack.workloads.len());
}
