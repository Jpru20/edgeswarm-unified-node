use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const CAPACITY_TEST_REQUEST_SCHEMA_V1: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapacityTestRequestV1 {
    schema_version: u8,
    requested_at_unix_ms: u64,
}

fn now_unix_ms_v1() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

pub fn capacity_test_request_path_v1() -> PathBuf {
    crate::adapters::app_data_dir()
        .join("capacity_test_request.json")
}

pub fn capacity_test_request_pending_v1() -> bool {
    capacity_test_request_path_v1().exists()
}

fn request_at_v1(path: &Path) -> Result<bool, String> {
    if path.exists() {
        return Ok(false);
    }

    let parent = path
        .parent()
        .ok_or_else(|| "capacity_test_request_parent_missing".to_string())?;

    fs::create_dir_all(parent)
        .map_err(|_| "capacity_test_request_directory_failed".to_string())?;

    let request = CapacityTestRequestV1 {
        schema_version: CAPACITY_TEST_REQUEST_SCHEMA_V1,
        requested_at_unix_ms: now_unix_ms_v1(),
    };

    let raw = serde_json::to_vec_pretty(&request)
        .map_err(|_| "capacity_test_request_serialize_failed".to_string())?;

    let temporary =
        path.with_extension(format!("tmp-{}", std::process::id()));

    fs::write(&temporary, raw)
        .map_err(|_| "capacity_test_request_write_failed".to_string())?;

    fs::rename(&temporary, path)
        .map_err(|_| "capacity_test_request_commit_failed".to_string())?;

    Ok(true)
}

pub fn request_capacity_test_v1() -> Result<bool, String> {
    request_at_v1(&capacity_test_request_path_v1())
}

fn take_at_v1(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }

    let raw = fs::read_to_string(path)
        .map_err(|_| "capacity_test_request_read_failed".to_string())?;

    let request: CapacityTestRequestV1 =
        serde_json::from_str(&raw)
            .map_err(|_| {
                let _ = fs::remove_file(path);
                "capacity_test_request_invalid".to_string()
            })?;

    if request.schema_version != CAPACITY_TEST_REQUEST_SCHEMA_V1 {
        let _ = fs::remove_file(path);
        return Err("capacity_test_request_schema_unsupported".into());
    }

    fs::remove_file(path)
        .map_err(|_| "capacity_test_request_remove_failed".to_string())?;

    Ok(true)
}

pub fn take_capacity_test_request_v1() -> Result<bool, String> {
    take_at_v1(&capacity_test_request_path_v1())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_test_request_round_trip_v1() {
        let path = std::env::temp_dir().join(format!(
            "edgeswarm-capacity-test-request-{}-{}.json",
            std::process::id(),
            now_unix_ms_v1()
        ));

        let _ = fs::remove_file(&path);

        assert!(request_at_v1(&path).unwrap());
        assert!(path.exists());

        assert!(!request_at_v1(&path).unwrap());

        assert!(take_at_v1(&path).unwrap());
        assert!(!path.exists());

        assert!(!take_at_v1(&path).unwrap());
    }
}
