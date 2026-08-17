use crate::core::{
    capacity::{CapacityBenchmarkSample, CapacityTaskResult},
    capacity_policy::CapacityPolicy,
    certification_workload::{CertificationPack, CertificationWorkload},
};
use serde::Serialize;

#[derive(Debug)]
pub struct ExecutionBatchResult {
    pub wall_time_ms: u64,
    pub peak_memory_bytes: Option<u64>,
    pub thermal_throttled: bool,
    pub task_results: Vec<CapacityTaskResult>,
}

pub trait CertificationExecutor {
    fn execute(
        &mut self,
        workloads: &[CertificationWorkload],
        concurrency: u16,
    ) -> Result<ExecutionBatchResult, String>;
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificationRunReport {
    pub certification_pack_id: String,
    pub tested_concurrency_levels: Vec<u16>,
    pub certified_concurrency: u16,
    pub rejected_concurrency: Option<u16>,
    pub samples: Vec<CapacityBenchmarkSample>,
}

pub struct CertificationRunner {
    policy: CapacityPolicy,
}

impl CertificationRunner {
    pub fn new(policy: CapacityPolicy) -> Self {
        Self { policy }
    }

    pub fn run<E: CertificationExecutor>(
        &self,
        pack: &CertificationPack,
        executor: &mut E,
    ) -> Result<CertificationRunReport, String> {
        pack.validate()?;

        let mut samples = Vec::new();
        let mut tested = Vec::new();
        let mut rejected = None;

        for concurrency in 1..=self.policy.maximum_concurrency {
            let batch = executor.execute(&pack.workloads, concurrency)?;

            if batch.task_results.len() != pack.workloads.len() {
                return Err(format!(
                    "executor_result_count_mismatch:expected={}:actual={}",
                    pack.workloads.len(),
                    batch.task_results.len()
                ));
            }

            let sample = build_sample(
                &pack.pack_id,
                concurrency,
                batch,
            );

            tested.push(concurrency);
            samples.push(sample);

            if concurrency > 1 {
                let recommended = self.policy.recommend(&samples);

                if recommended < concurrency {
                    rejected = Some(concurrency);
                    break;
                }
            }
        }

        let certified = self.policy.recommend(&samples);

        Ok(CertificationRunReport {
            certification_pack_id: pack.pack_id.clone(),
            tested_concurrency_levels: tested,
            certified_concurrency: certified,
            rejected_concurrency: rejected,
            samples,
        })
    }
}

fn build_sample(
    pack_id: &str,
    concurrency: u16,
    batch: ExecutionBatchResult,
) -> CapacityBenchmarkSample {
    let successful_tasks = batch
        .task_results
        .iter()
        .filter(|r| r.success)
        .count() as u16;

    let failed_tasks =
        batch.task_results.len() as u16 - successful_tasks;

    let valid_outputs = batch
        .task_results
        .iter()
        .filter(|r| r.output_valid)
        .count() as u16;

    let quality_pass_rate = if batch.task_results.is_empty() {
        0.0
    } else {
        valid_outputs as f64 / batch.task_results.len() as f64
    };

    let output_tokens: u64 = batch
        .task_results
        .iter()
        .map(|r| r.output_tokens as u64)
        .sum();

    let aggregate_tokens_per_second =
        if batch.wall_time_ms == 0 {
            0.0
        } else {
            output_tokens as f64 /
                (batch.wall_time_ms as f64 / 1000.0)
        };

    let median_task_wall_time_ms = median_u64(
        batch.task_results.iter().map(|r| r.wall_time_ms).collect()
    );

    let median_first_token_ms = median_optional_u64(
        batch.task_results
            .iter()
            .filter_map(|r| r.first_token_ms)
            .collect()
    );

    CapacityBenchmarkSample {
        certification_pack_id: pack_id.to_string(),
        concurrency,
        wall_time_ms: batch.wall_time_ms,
        median_task_wall_time_ms,
        median_first_token_ms,
        aggregate_tokens_per_second,
        peak_memory_bytes: batch.peak_memory_bytes,
        thermal_throttled: batch.thermal_throttled,
        successful_tasks,
        failed_tasks,
        valid_outputs,
        quality_pass_rate,
        task_results: batch.task_results,
    }
}

fn median_u64(mut values: Vec<u64>) -> u64 {
    if values.is_empty() {
        return 0;
    }

    values.sort_unstable();
    let mid = values.len() / 2;

    if values.len() % 2 == 0 {
        ((values[mid - 1] as u128 + values[mid] as u128) / 2) as u64
    } else {
        values[mid]
    }
}

fn median_optional_u64(values: Vec<u64>) -> Option<u64> {
    if values.is_empty() {
        None
    } else {
        Some(median_u64(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        capacity::CapacityTaskResult,
        certification_workload::{
            built_in_3b_realworld_v1,
            CertificationProfile,
        },
    };

    struct ReferenceThreeBExecutor;

    impl CertificationExecutor for ReferenceThreeBExecutor {
        fn execute(
            &mut self,
            workloads: &[CertificationWorkload],
            concurrency: u16,
        ) -> Result<ExecutionBatchResult, String> {
            let (pack_wall, task_wall, ttft) = match concurrency {
                1 => (60_000, 6_600, 300),
                2 => (40_000, 8_490, 380),
                3 => (36_000, 11_470, 520),
                _ => (36_000, 15_000, 700),
            };

            let task_results = workloads
                .iter()
                .map(|workload| CapacityTaskResult {
                    workload_id: workload.id.clone(),
                    profile: match workload.profile {
                        CertificationProfile::Normal =>
                            CertificationProfile::Normal,
                        CertificationProfile::Heavy =>
                            CertificationProfile::Heavy,
                        CertificationProfile::Structured =>
                            CertificationProfile::Structured,
                    },
                    success: true,
                    output_valid: true,
                    wall_time_ms: task_wall,
                    first_token_ms: Some(ttft),
                    output_tokens: 200,
                    tokens_per_second:
                        200.0 / (task_wall as f64 / 1000.0),
                })
                .collect();

            Ok(ExecutionBatchResult {
                wall_time_ms: pack_wall,
                peak_memory_bytes: Some(4_000_000_000),
                thermal_throttled: false,
                task_results,
            })
        }
    }

    #[test]
    fn runner_stops_after_rejecting_third_slot() {
        let pack = built_in_3b_realworld_v1().unwrap();
        let mut executor = ReferenceThreeBExecutor;

        let report = CertificationRunner::new(
            CapacityPolicy::default()
        )
        .run(&pack, &mut executor)
        .unwrap();

        assert_eq!(report.tested_concurrency_levels, vec![1, 2, 3]);
        assert_eq!(report.certified_concurrency, 2);
        assert_eq!(report.rejected_concurrency, Some(3));
    }
}
