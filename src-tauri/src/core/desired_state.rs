use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DESIRED_STATE_SCHEMA_VERSION_V1: u8 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesiredNodeStateV1 {
    Running,
    UserStopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DesiredNodeStateRecordV1 {
    pub schema_version: u8,
    pub desired_state: DesiredNodeStateV1,
    pub changed_at_unix_ms: u64,
    pub reason: String,
}

fn now_unix_ms_v1() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn fail_closed_record_v1(reason: &str) -> DesiredNodeStateRecordV1 {
    DesiredNodeStateRecordV1 {
        schema_version: DESIRED_STATE_SCHEMA_VERSION_V1,
        desired_state: DesiredNodeStateV1::UserStopped,
        changed_at_unix_ms: now_unix_ms_v1(),
        reason: reason.to_string(),
    }
}

pub fn desired_state_path_v1() -> PathBuf {
    crate::adapters::app_data_dir().join("desired_state.json")
}

fn persist_at_v1(
    path: &Path,
    desired_state: DesiredNodeStateV1,
    reason: &str,
) -> Result<DesiredNodeStateRecordV1, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("desired_state_directory_failed:{e}"))?;
    }

    let record = DesiredNodeStateRecordV1 {
        schema_version: DESIRED_STATE_SCHEMA_VERSION_V1,
        desired_state,
        changed_at_unix_ms: now_unix_ms_v1(),
        reason: reason.to_string(),
    };

    let raw = serde_json::to_vec_pretty(&record)
        .map_err(|e| format!("desired_state_serialize_failed:{e}"))?;

    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        now_unix_ms_v1()
    ));

    fs::write(&temporary, raw)
        .map_err(|e| format!("desired_state_write_failed:{e}"))?;

    #[cfg(target_os = "windows")]
    if path.exists() {
        fs::remove_file(path)
            .map_err(|e| format!("desired_state_replace_failed:{e}"))?;
    }

    fs::rename(&temporary, path)
        .map_err(|e| format!("desired_state_commit_failed:{e}"))?;

    Ok(record)
}

pub fn persist_desired_node_state_v1(
    desired_state: DesiredNodeStateV1,
    reason: &str,
) -> Result<DesiredNodeStateRecordV1, String> {
    persist_at_v1(
        &desired_state_path_v1(),
        desired_state,
        reason,
    )
}

fn load_at_v1(
    path: &Path,
) -> Result<DesiredNodeStateRecordV1, String> {
    if !path.exists() {
        return Ok(fail_closed_record_v1(
            "state_missing_fail_closed",
        ));
    }

    let raw = fs::read_to_string(path)
        .map_err(|e| format!("desired_state_read_failed:{e}"))?;

    let record: DesiredNodeStateRecordV1 =
        serde_json::from_str(&raw)
            .map_err(|e| format!("desired_state_parse_failed:{e}"))?;

    if record.schema_version != DESIRED_STATE_SCHEMA_VERSION_V1 {
        return Err("desired_state_schema_unsupported".into());
    }

    Ok(record)
}

pub fn load_desired_node_state_v1(
) -> Result<DesiredNodeStateRecordV1, String> {
    load_at_v1(&desired_state_path_v1())
}

pub fn effective_desired_node_state_v1(
) -> DesiredNodeStateRecordV1 {
    load_desired_node_state_v1().unwrap_or_else(|_| {
        fail_closed_record_v1("state_invalid_fail_closed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "edgeswarm-{name}-{}-{}.json",
            std::process::id(),
            now_unix_ms_v1()
        ))
    }

    #[test]
    fn missing_state_fails_closed_v1() {
        let path = test_path("desired-missing");
        let _ = fs::remove_file(&path);

        let state = load_at_v1(&path).unwrap();

        assert_eq!(
            state.desired_state,
            DesiredNodeStateV1::UserStopped
        );
    }

    #[test]
    fn desired_state_round_trip_v1() {
        let path = test_path("desired-roundtrip");

        persist_at_v1(
            &path,
            DesiredNodeStateV1::Running,
            "user_start",
        )
        .unwrap();

        let running = load_at_v1(&path).unwrap();

        assert_eq!(
            running.desired_state,
            DesiredNodeStateV1::Running
        );

        persist_at_v1(
            &path,
            DesiredNodeStateV1::UserStopped,
            "user_stop",
        )
        .unwrap();

        let stopped = load_at_v1(&path).unwrap();

        assert_eq!(
            stopped.desired_state,
            DesiredNodeStateV1::UserStopped
        );

        let _ = fs::remove_file(path);
    }
}