use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificationProgressV1 {
    pub state: String,
    pub maximum_concurrency: u16,
    pub current_concurrency: Option<u16>,
    pub completed_workloads: usize,
    pub total_workloads: usize,
    pub tested_concurrency_levels: Vec<u16>,
    pub certified_concurrency: u16,
    pub rejected_concurrency: Option<u16>,
    pub last_error: Option<String>,
}

impl Default for CertificationProgressV1 {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            maximum_concurrency: 1,
            current_concurrency: None,
            completed_workloads: 0,
            total_workloads: 0,
            tested_concurrency_levels: Vec::new(),
            certified_concurrency: 0,
            rejected_concurrency: None,
            last_error: None,
        }
    }
}

static CERTIFICATION_PROGRESS_V1:
    OnceLock<Mutex<CertificationProgressV1>> =
    OnceLock::new();

fn store_v1() -> &'static Mutex<CertificationProgressV1> {
    CERTIFICATION_PROGRESS_V1.get_or_init(|| {
        Mutex::new(CertificationProgressV1::default())
    })
}

pub fn certification_progress_v1() -> CertificationProgressV1 {
    store_v1()
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default()
}

pub fn certification_begin_v1(
    maximum_concurrency: u16,
    total_workloads: usize,
) {
    if let Ok(mut value) = store_v1().lock() {
        *value = CertificationProgressV1 {
            state: "running".into(),
            maximum_concurrency:
                maximum_concurrency.max(1),
            current_concurrency: None,
            completed_workloads: 0,
            total_workloads,
            tested_concurrency_levels: Vec::new(),
            certified_concurrency: 0,
            rejected_concurrency: None,
            last_error: None,
        };
    }
}

pub fn certification_level_started_v1(
    concurrency: u16,
    total_workloads: usize,
) {
    if let Ok(mut value) = store_v1().lock() {
        value.state = "running".into();
        value.current_concurrency = Some(concurrency);
        value.completed_workloads = 0;
        value.total_workloads = total_workloads;
        value.last_error = None;
    }
}

pub fn certification_workload_progress_v1(
    concurrency: u16,
    completed: usize,
    total: usize,
) {
    if let Ok(mut value) = store_v1().lock() {
        if value.current_concurrency == Some(concurrency) {
            value.completed_workloads =
                completed.min(total);
            value.total_workloads = total;
        }
    }
}

pub fn certification_level_passed_v1(
    concurrency: u16,
) {
    if let Ok(mut value) = store_v1().lock() {
        if !value
            .tested_concurrency_levels
            .contains(&concurrency)
        {
            value
                .tested_concurrency_levels
                .push(concurrency);
        }

        value.certified_concurrency =
            value.certified_concurrency.max(concurrency);

        value.current_concurrency = None;
    }
}

pub fn certification_level_rejected_v1(
    concurrency: u16,
    reason: Option<String>,
) {
    if let Ok(mut value) = store_v1().lock() {
        value.rejected_concurrency =
            Some(concurrency);

        value.current_concurrency = None;
        value.last_error = reason;
    }
}

pub fn certification_complete_v1(
    certified_concurrency: u16,
    rejected_concurrency: Option<u16>,
    tested_levels: Vec<u16>,
) {
    if let Ok(mut value) = store_v1().lock() {
        value.state = "complete".into();
        value.current_concurrency = None;
        value.completed_workloads =
            value.total_workloads;
        value.tested_concurrency_levels =
            tested_levels;
        value.certified_concurrency =
            certified_concurrency;
        value.rejected_concurrency =
            rejected_concurrency;
        value.last_error = None;
    }
}

pub fn certification_error_v1(
    error: impl Into<String>,
) {
    if let Ok(mut value) = store_v1().lock() {
        value.state = "error".into();
        value.current_concurrency = None;
        value.last_error = Some(error.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certification_progress_lifecycle_v1() {
        certification_begin_v1(4, 6);

        certification_level_started_v1(1, 6);
        certification_workload_progress_v1(1, 4, 6);

        let running = certification_progress_v1();

        assert_eq!(running.state, "running");
        assert_eq!(running.current_concurrency, Some(1));
        assert_eq!(running.completed_workloads, 4);

        certification_level_passed_v1(1);
        certification_level_started_v1(2, 6);
        certification_workload_progress_v1(2, 6, 6);
        certification_level_passed_v1(2);

        certification_level_rejected_v1(
            3,
            Some("latency_gate_failed".into()),
        );

        certification_complete_v1(
            2,
            Some(3),
            vec![1, 2, 3],
        );

        let complete = certification_progress_v1();

        assert_eq!(complete.state, "complete");
        assert_eq!(complete.certified_concurrency, 2);
        assert_eq!(complete.rejected_concurrency, Some(3));
    }
}
