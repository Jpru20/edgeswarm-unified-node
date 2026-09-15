use crate::core::{
    capacity_test_control::capacity_test_request_pending_v1,
    certification_progress::{
        certification_progress_v1,
        CertificationProgressV1,
    },
    model_provisioning::{
        model_download_progress_v1,
        ModelDownloadProgressV1,
    },
    node_service::node_service_logs,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const NODE_STATUS_BRIDGE_SCHEMA_V1: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatusBridgeV1 {
    pub schema_version: u8,
    pub writer_pid: u32,
    pub updated_at_unix_ms: u64,
    pub running: bool,
    pub stopping: bool,
    pub last_error: Option<String>,
    pub logs: Vec<String>,
    pub model_download: Option<ModelDownloadProgressV1>,

    #[serde(default)]
    pub certification: CertificationProgressV1,

    #[serde(default)]
    pub capacity_test_requested: bool,
}

fn now_unix_ms_v1() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

pub fn node_status_bridge_path_v1() -> PathBuf {
    crate::adapters::app_data_dir()
        .join("node_status.json")
}

pub fn current_node_status_snapshot_v1(
    running: bool,
    stopping: bool,
    last_error: Option<String>,
) -> NodeStatusBridgeV1 {
    NodeStatusBridgeV1 {
        schema_version: NODE_STATUS_BRIDGE_SCHEMA_V1,
        writer_pid: std::process::id(),
        updated_at_unix_ms: now_unix_ms_v1(),
        running,
        stopping,
        last_error,
        logs: node_service_logs(),
        model_download: model_download_progress_v1(),
        certification: certification_progress_v1(),
        capacity_test_requested: capacity_test_request_pending_v1(),
    }
}

fn persist_at_v1(
    path: &Path,
    snapshot: &NodeStatusBridgeV1,
) -> Result<(), String> {
    let parent =
        path.parent()
            .ok_or_else(|| {
                "node_status_parent_missing".to_string()
            })?;

    fs::create_dir_all(parent)
        .map_err(|_| {
            "node_status_directory_failed".to_string()
        })?;

    let raw =
        serde_json::to_vec_pretty(snapshot)
            .map_err(|_| {
                "node_status_serialize_failed".to_string()
            })?;

    let temporary =
        path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            now_unix_ms_v1()
        ));

    fs::write(&temporary, raw)
        .map_err(|_| {
            "node_status_write_failed".to_string()
        })?;

    #[cfg(target_os = "windows")]
    if path.exists() {
        fs::remove_file(path)
            .map_err(|_| {
                "node_status_replace_failed".to_string()
            })?;
    }

    fs::rename(&temporary, path)
        .map_err(|_| {
            "node_status_commit_failed".to_string()
        })?;

    Ok(())
}

pub fn publish_node_status_v1(
    running: bool,
    stopping: bool,
    last_error: Option<String>,
) -> Result<(), String> {
    let snapshot =
        current_node_status_snapshot_v1(
            running,
            stopping,
            last_error,
        );

    persist_at_v1(
        &node_status_bridge_path_v1(),
        &snapshot,
    )
}

fn load_at_v1(
    path: &Path,
) -> Result<NodeStatusBridgeV1, String> {
    let raw =
        fs::read_to_string(path)
            .map_err(|_| {
                "node_status_read_failed".to_string()
            })?;

    let value: NodeStatusBridgeV1 =
        serde_json::from_str(&raw)
            .map_err(|_| {
                "node_status_parse_failed".to_string()
            })?;

    if value.schema_version
        != NODE_STATUS_BRIDGE_SCHEMA_V1
    {
        return Err(
            "node_status_schema_unsupported".into()
        );
    }

    Ok(value)
}

pub fn load_node_status_v1(
) -> Result<NodeStatusBridgeV1, String> {
    load_at_v1(
        &node_status_bridge_path_v1()
    )
}

pub fn load_fresh_node_status_v1(
    max_age_ms: u64,
) -> Result<Option<NodeStatusBridgeV1>, String> {
    let value =
        load_node_status_v1()?;

    let age =
        now_unix_ms_v1()
            .saturating_sub(
                value.updated_at_unix_ms
            );

    if age > max_age_ms {
        return Ok(None);
    }

    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path_v1(
        name: &str,
    ) -> PathBuf {
        std::env::temp_dir().join(
            format!(
                "edgeswarm-node-status-{name}-{}-{}.json",
                std::process::id(),
                now_unix_ms_v1()
            )
        )
    }

    #[test]
    fn status_bridge_round_trip_v1() {
        let path =
            test_path_v1("roundtrip");

        let snapshot =
            NodeStatusBridgeV1 {
                schema_version:
                    NODE_STATUS_BRIDGE_SCHEMA_V1,
                writer_pid: 123,
                updated_at_unix_ms:
                    now_unix_ms_v1(),
                running: true,
                stopping: false,
                last_error: None,
                logs: vec![
                    "LOCAL_RUNTIME_READY=true".into()
                ],
                model_download: None,
                certification:
                    CertificationProgressV1::default(),
                capacity_test_requested: false,
            };

        persist_at_v1(
            &path,
            &snapshot
        ).unwrap();

        let loaded =
            load_at_v1(&path).unwrap();

        assert!(loaded.running);
        assert!(!loaded.stopping);
        assert_eq!(
            loaded.writer_pid,
            123
        );
        assert_eq!(
            loaded.logs,
            vec![
                "LOCAL_RUNTIME_READY=true"
                    .to_string()
            ]
        );

        let _ =
            fs::remove_file(path);
    }

    #[test]
    fn invalid_status_bridge_fails_closed_v1() {
        let path =
            test_path_v1("invalid");

        fs::write(
            &path,
            b"{invalid-json"
        ).unwrap();

        assert!(
            load_at_v1(&path)
                .is_err()
        );

        let _ =
            fs::remove_file(path);
    }
}
