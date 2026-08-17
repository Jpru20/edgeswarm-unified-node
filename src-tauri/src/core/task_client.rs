use crate::core::result_signing;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEnvelope {
    pub task_id: Value,

    #[serde(default)]
    pub client_name: Option<String>,

    #[serde(default)]
    pub prompt: String,

    #[serde(default)]
    pub required_model: Option<String>,

    #[serde(default)]
    pub selected_model: Option<String>,

    #[serde(default)]
    pub model_route_reason: Option<String>,

    #[serde(default)]
    pub model_routing_version: Option<String>,

    #[serde(default)]
    pub verification_seed: Option<Value>,

    #[serde(default)]
    pub checkpoint_indices: Vec<Value>,

    #[serde(default)]
    pub verification_method: Option<String>,

    #[serde(default)]
    pub max_output_tokens: Option<u64>,
}

impl TaskEnvelope {
    pub fn task_id_text(&self) -> String {
        match &self.task_id {
            Value::String(value) => value.clone(),
            value => value.to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetJobsResponse {
    #[serde(default)]
    pub task: Option<TaskEnvelope>,

    #[serde(default)]
    pub tasks: Vec<TaskEnvelope>,

    #[serde(default)]
    pub blocked: bool,

    #[serde(default)]
    pub block_reason: Option<String>,

    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultPayload {
    pub task_id: Value,
    pub worker: String,
    pub provider_email: String,
    pub score: u16,
    pub signature: String,
    pub hardware_id: String,
    pub ai_output: String,
    pub ai_translation: Option<String>,
    pub status: String,

    #[serde(rename = "latency_ms")]
    pub latency_ms: u64,

    pub required_model: String,
    pub model_id_used: String,
    pub runtime: String,
    pub runtime_acceleration: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResultEnvelope {
    pub file_hash: String,
    pub payload: ResultPayload,
}

pub struct TaskClientDryRun {
    pub poll_endpoint_valid: bool,
    pub submit_endpoint_valid: bool,
    pub hardware_id_present: bool,
    pub provider_email_present: bool,
    pub capability_count: usize,
    pub limit: u16,
    pub network_request_sent: bool,
}

pub fn build_poll_url(
    hardware_id: &str,
    provider_email: &str,
    capabilities: &[String],
    version: &str,
    platform: &str,
) -> Result<Url, String> {
    let base = env::var("GCP_BASE_URL")
        .unwrap_or_else(|_| "https://api.edgeswarm.io".into())
        .trim_end_matches('/')
        .to_string();

    let mut url = Url::parse(
        &format!("{base}/swarm/get-jobs")
    )
    .map_err(|_| "get_jobs_url_invalid".to_string())?;

    url.query_pairs_mut()
        .append_pair("hardwareId", hardware_id)
        .append_pair("providerEmail", provider_email)
        .append_pair("capabilities", &capabilities.join(","))
        .append_pair("limit", "1")
        .append_pair("version", version)
        .append_pair("appType", "cross-platform-node")
        .append_pair("platform", platform);

    Ok(url)
}

pub fn build_submit_result(
    task: &TaskEnvelope,
    ai_output: &str,
    provider_email: &str,
    worker: &str,
    hardware_id: &str,
    private_key: &str,
    latency_ms: u64,
    model_id_used: &str,
    runtime: &str,
    runtime_acceleration: &str,
) -> Result<SubmitResultEnvelope, String> {
    let file_hash = Sha256::digest(ai_output.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let score = 100;
    let task_id = task.task_id_text();

    let signature = result_signing::sign_result(
        &task_id,
        score,
        &file_hash,
        hardware_id,
        private_key,
    )?;

    Ok(SubmitResultEnvelope {
        file_hash,
        payload: ResultPayload {
            task_id: task.task_id.clone(),
            worker: worker.to_string(),
            provider_email: provider_email.to_lowercase(),
            score,
            signature,
            hardware_id: hardware_id.to_string(),
            ai_output: ai_output.to_string(),
            ai_translation: None,
            status: "success".into(),
            latency_ms,
            required_model: task
                .required_model
                .clone()
                .unwrap_or_default(),
            model_id_used: model_id_used.to_string(),
            runtime: runtime.to_string(),
            runtime_acceleration:
                runtime_acceleration.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_production_task_envelope() {
        let parsed: GetJobsResponse =
            serde_json::from_str(
                r#"{
                    "task":{
                        "taskId":2001,
                        "prompt":"Say hello",
                        "requiredModel":"Neural-Inference-3B",
                        "selectedModel":"qwen2.5:3b",
                        "maxOutputTokens":128
                    },
                    "tasks":[{
                        "taskId":2001,
                        "prompt":"Say hello",
                        "requiredModel":"Neural-Inference-3B",
                        "selectedModel":"qwen2.5:3b",
                        "maxOutputTokens":128
                    }]
                }"#,
            )
            .unwrap();

        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(parsed.tasks[0].task_id_text(), "2001");
    }

    #[test]
    fn builds_signed_result_envelope() {
        let task = TaskEnvelope {
            task_id: Value::from(2001),
            client_name: None,
            prompt: "Say hello".into(),
            required_model:
                Some("Neural-Inference-3B".into()),
            selected_model:
                Some("qwen2.5:3b".into()),
            model_route_reason: None,
            model_routing_version: None,
            verification_seed: None,
            checkpoint_indices: vec![],
            verification_method: None,
            max_output_tokens: Some(128),
        };

        let result = build_submit_result(
            &task,
            r#"{"response":"hello"}"#,
            "test@example.com",
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf",
            "6957eb286fb15a2813ce44699232d053d69d91be307fdf0018df1001b4eda5de",
            "0000000000000000000000000000000000000000000000000000000000000001",
            100,
            "qwen2.5:3b",
            "llama.cpp",
            "cpu",
        )
        .unwrap();

        assert_eq!(result.file_hash.len(), 64);
        assert_eq!(result.payload.signature.len(), 132);
        assert_eq!(result.payload.score, 100);
    }
}
