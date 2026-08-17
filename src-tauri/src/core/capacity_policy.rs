use crate::core::capacity::CapacityBenchmarkSample;

#[derive(Debug, Clone)]
pub struct CapacityPolicy {
    pub required_workloads: usize,
    pub minimum_quality_pass_rate: f64,
    pub minimum_incremental_throughput_gain: f64,
    pub maximum_median_latency_multiplier: f64,
    pub maximum_ttft_multiplier: f64,
    pub maximum_concurrency: u16,
}

impl Default for CapacityPolicy {
    fn default() -> Self {
        Self {
            required_workloads: 6,
            minimum_quality_pass_rate: 1.0,
            minimum_incremental_throughput_gain: 0.15,
            maximum_median_latency_multiplier: 1.50,
            maximum_ttft_multiplier: 1.75,
            maximum_concurrency: 5,
        }
    }
}

impl CapacityPolicy {
    fn sample_is_reliable(&self, sample: &CapacityBenchmarkSample) -> bool {
        let expected = self.required_workloads as u16;

        sample.task_results.len() == self.required_workloads
            && sample.successful_tasks == expected
            && sample.failed_tasks == 0
            && sample.valid_outputs == expected
            && sample.quality_pass_rate >= self.minimum_quality_pass_rate
            && !sample.thermal_throttled
            && sample.aggregate_tokens_per_second > 0.0
            && sample.median_task_wall_time_ms > 0
            && sample
                .task_results
                .iter()
                .all(|result| result.success && result.output_valid)
    }

    pub fn recommend(&self, samples: &[CapacityBenchmarkSample]) -> u16 {
        let Some(baseline) = samples
            .iter()
            .find(|sample| sample.concurrency == 1)
        else {
            return 1;
        };

        if !self.sample_is_reliable(baseline) {
            return 1;
        }

        let baseline_latency = baseline.median_task_wall_time_ms as f64;
        let baseline_ttft = baseline.median_first_token_ms;
        let mut previous_throughput = baseline.aggregate_tokens_per_second;
        let mut certified = 1;

        for concurrency in 2..=self.maximum_concurrency {
            let Some(candidate) = samples
                .iter()
                .find(|sample| sample.concurrency == concurrency)
            else {
                break;
            };

            if !self.sample_is_reliable(candidate) {
                break;
            }

            let incremental_gain =
                candidate.aggregate_tokens_per_second / previous_throughput - 1.0;

            let latency_multiplier =
                candidate.median_task_wall_time_ms as f64 / baseline_latency;

            let ttft_ok = match (
                baseline_ttft,
                candidate.median_first_token_ms,
            ) {
                (Some(base), Some(current)) if base > 0 => {
                    current as f64 / base as f64
                        <= self.maximum_ttft_multiplier
                }
                _ => true,
            };

            if incremental_gain < self.minimum_incremental_throughput_gain
                || latency_multiplier > self.maximum_median_latency_multiplier
                || !ttft_ok
            {
                break;
            }

            certified = concurrency;
            previous_throughput = candidate.aggregate_tokens_per_second;
        }

        certified
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        capacity::CapacityTaskResult,
        certification_workload::CertificationProfile,
    };

    fn task_result(id: usize) -> CapacityTaskResult {
        CapacityTaskResult {
            workload_id: format!("workload-{id}"),
            profile: match id % 3 {
                0 => CertificationProfile::Normal,
                1 => CertificationProfile::Heavy,
                _ => CertificationProfile::Structured,
            },
            success: true,
            output_valid: true,
            wall_time_ms: 7000,
            first_token_ms: Some(300),
            output_tokens: 256,
            tokens_per_second: 20.0,
        }
    }

    fn sample(
        concurrency: u16,
        median_latency_ms: u64,
        ttft_ms: u64,
        throughput: f64,
    ) -> CapacityBenchmarkSample {
        CapacityBenchmarkSample {
            certification_pack_id: "edgeswarm-3b-realworld-v1".into(),
            concurrency,
            wall_time_ms: 40000,
            median_task_wall_time_ms: median_latency_ms,
            median_first_token_ms: Some(ttft_ms),
            aggregate_tokens_per_second: throughput,
            peak_memory_bytes: Some(4_000_000_000),
            thermal_throttled: false,
            successful_tasks: 6,
            failed_tasks: 0,
            valid_outputs: 6,
            quality_pass_rate: 1.0,
            task_results: (0..6).map(task_result).collect(),
        }
    }

    #[test]
    fn realistic_three_b_example_certifies_two_slots() {
        let samples = vec![
            sample(1, 6600, 300, 20.2),
            sample(2, 8490, 380, 30.2),
            sample(3, 11470, 520, 33.5),
        ];

        assert_eq!(CapacityPolicy::default().recommend(&samples), 2);
    }

    #[test]
    fn quality_failure_blocks_higher_concurrency() {
        let baseline = sample(1, 6600, 300, 20.2);
        let mut overloaded = sample(2, 8000, 360, 35.0);

        overloaded.valid_outputs = 5;
        overloaded.quality_pass_rate = 5.0 / 6.0;
        overloaded.task_results[5].output_valid = false;

        assert_eq!(
            CapacityPolicy::default().recommend(&[baseline, overloaded]),
            1
        );
    }

    #[test]
    fn thermal_throttling_blocks_higher_concurrency() {
        let baseline = sample(1, 6600, 300, 20.2);
        let mut hot = sample(2, 8000, 360, 35.0);

        hot.thermal_throttled = true;

        assert_eq!(
            CapacityPolicy::default().recommend(&[baseline, hot]),
            1
        );
    }
}
