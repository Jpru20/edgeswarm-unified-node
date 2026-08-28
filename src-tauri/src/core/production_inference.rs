use crate::core::generation_policy::production_generation_settings_v1;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct ProductionInferenceResult {
    pub ai_output: String,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tokens_per_second: f64,
    pub prompt_eval_ms: Option<u64>,
    pub generation_ms: Option<u64>,
    pub max_tokens: u32,
}

pub struct ProductionLlamaClient {
    http: Client,
    base_url: String,
}

impl ProductionLlamaClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, String> {
        let http = Client::builder()
            .timeout(Duration::from_secs(105))
            .no_proxy()
            .build()
            .map_err(|_| "production_llama_client_build_failed".to_string())?;

        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }

    pub fn health_check(&self) -> Result<(), String> {
        let response = self
            .http
            .get(format!("{}/health", self.base_url))
            .send()
            .map_err(|_| "production_llama_health_request_failed".to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "production_llama_not_ready_http_{}",
                response.status().as_u16()
            ));
        }

        Ok(())
    }

    // TRUE_NODE_CHUNK_STREAMING_V1
    // Reads llama.cpp OpenAI-compatible SSE deltas while preserving
    // the same complete final result used by normal consensus.
    pub fn execute_streaming<F>(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
        mut on_chunk: F,
    ) -> Result<ProductionInferenceResult, String>
    where
        F: FnMut(&str),
    {
        let prompt = prompt.trim();

        if prompt.is_empty() {
            return Err("production_task_prompt_empty".into());
        }

        let requested = max_output_tokens.unwrap_or(512).max(1).min(4096) as u32;

        let generation = production_generation_settings_v1(prompt, None, Some(requested));

        let max_tokens = generation.max_tokens.max(1).min(4096);

        let body = json!({
            "model": "local-model",
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": generation.temperature,
            "top_p": generation.top_p,
            "max_tokens": max_tokens,
            "stop": generation.stop,
            "stream": true,
            "stream_options": {
                "include_usage": true
            }
        });

        let started = Instant::now();

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", "Bearer no-key")
            .json(&body)
            .send()
            .map_err(|_| "production_llama_stream_request_failed".to_string())?;

        let status = response.status();

        if !status.is_success() {
            return Err(format!("production_llama_http_{}", status.as_u16()));
        }

        let reader = BufReader::new(response);

        let mut output = String::new();

        let mut input_tokens = 0_u64;
        let mut output_tokens = 0_u64;
        let mut runtime_tps = None::<f64>;
        let mut prompt_eval_ms = None::<u64>;
        let mut generation_ms = None::<u64>;

        for line_result in reader.lines() {
            let line =
                line_result.map_err(|_| "production_llama_stream_read_failed".to_string())?;

            let line = line.trim();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };

            let data = data.trim();

            if data == "[DONE]" {
                break;
            }

            let parsed: Value = serde_json::from_str(data)
                .map_err(|_| "production_llama_stream_json_failed".to_string())?;

            let delta = parsed
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
                .or_else(|| parsed.pointer("/choices/0/text").and_then(Value::as_str))
                .unwrap_or("");

            if !delta.is_empty() {
                output.push_str(delta);
                on_chunk(delta);
            }

            if let Some(value) = parsed
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64)
                .or_else(|| parsed.pointer("/timings/prompt_n").and_then(Value::as_u64))
            {
                input_tokens = value;
            }

            if let Some(value) = parsed
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_u64)
                .or_else(|| {
                    parsed
                        .pointer("/timings/predicted_n")
                        .and_then(Value::as_u64)
                })
            {
                output_tokens = value;
            }

            if let Some(value) = parsed
                .pointer("/timings/predicted_per_second")
                .and_then(Value::as_f64)
            {
                runtime_tps = Some(value);
            }

            if let Some(value) = parsed.pointer("/timings/prompt_ms").and_then(Value::as_f64) {
                prompt_eval_ms = Some(value.round() as u64);
            }

            if let Some(value) = parsed
                .pointer("/timings/predicted_ms")
                .and_then(Value::as_f64)
            {
                generation_ms = Some(value.round() as u64);
            }
        }

        let output = output.trim();

        if output.is_empty() {
            return Err("production_llama_empty_output".into());
        }

        let ai_output = if output.starts_with('{') {
            match serde_json::from_str::<Value>(output) {
                Ok(Value::Object(object)) => serde_json::to_string(&Value::Object(object))
                    .map_err(|_| "production_output_encode_failed".to_string())?,

                _ => json!({
                    "response": output
                })
                .to_string(),
            }
        } else {
            json!({
                "response": output
            })
            .to_string()
        };

        let latency_ms = started.elapsed().as_millis() as u64;

        if output_tokens == 0 {
            output_tokens = ((output.chars().count() as u64) + 3) / 4;
        }

        let tokens_per_second = runtime_tps.unwrap_or_else(|| {
            if latency_ms > 0 {
                output_tokens as f64 / (latency_ms as f64 / 1000.0)
            } else {
                0.0
            }
        });

        Ok(ProductionInferenceResult {
            ai_output,
            latency_ms,
            input_tokens,
            output_tokens,
            tokens_per_second,
            prompt_eval_ms,
            generation_ms,
            max_tokens,
        })
    }

    pub fn execute(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
    ) -> Result<ProductionInferenceResult, String> {
        let prompt = prompt.trim();

        if prompt.is_empty() {
            return Err("production_task_prompt_empty".into());
        }

        let requested = max_output_tokens.unwrap_or(512).max(1).min(4096) as u32;

        let generation = production_generation_settings_v1(prompt, None, Some(requested));

        let max_tokens = generation.max_tokens.max(1).min(4096);

        let mut body = json!({
            "model": "local-model",
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ],
            "temperature": generation.temperature,
            "top_p": generation.top_p,
            "max_tokens": max_tokens,
            "stop": generation.stop,
            "stream": false
        });

        // JSON_CONSTRAINED_GENERATION_V1
        // Keep live customer execution aligned with certification:
        // JSON-mode tasks use llama.cpp constrained JSON generation.
        if generation.mode == "json" {
            body["json_schema"] = json!({
                "type": "object"
            });
        }

        let started = Instant::now();

        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", "Bearer no-key")
            .json(&body)
            .send()
            .map_err(|_| "production_llama_request_failed".to_string())?;

        let status = response.status();

        let raw = response
            .text()
            .map_err(|_| "production_llama_response_read_failed".to_string())?;

        if !status.is_success() {
            return Err(format!("production_llama_http_{}", status.as_u16()));
        }

        let parsed: Value = serde_json::from_str(&raw)
            .map_err(|_| "production_llama_response_invalid".to_string())?;

        let output = parsed
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();

        if output.is_empty() {
            return Err("production_llama_empty_output".into());
        }

        let ai_output = if generation.mode == "json" {
            match serde_json::from_str::<Value>(output) {
                Ok(Value::Object(object)) => serde_json::to_string(&Value::Object(object))
                    .map_err(|_| "production_output_encode_failed".to_string())?,
                Ok(_) => {
                    return Err("production_json_output_not_object".into());
                }
                Err(_) => {
                    return Err("production_json_output_invalid".into());
                }
            }
        } else if output.starts_with('{') {
            match serde_json::from_str::<Value>(output) {
                Ok(Value::Object(object)) => serde_json::to_string(&Value::Object(object))
                    .map_err(|_| "production_output_encode_failed".to_string())?,
                _ => json!({"response": output}).to_string(),
            }
        } else {
            json!({"response": output}).to_string()
        };

        let latency_ms = started.elapsed().as_millis() as u64;

        let input_tokens = parsed
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
            .or_else(|| parsed.pointer("/timings/prompt_n").and_then(Value::as_u64))
            .unwrap_or(0);

        let output_tokens = parsed
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .or_else(|| {
                parsed
                    .pointer("/timings/predicted_n")
                    .and_then(Value::as_u64)
            })
            .unwrap_or(0);

        let tokens_per_second = parsed
            .pointer("/timings/predicted_per_second")
            .and_then(Value::as_f64)
            .unwrap_or_else(|| {
                if latency_ms > 0 {
                    output_tokens as f64 / (latency_ms as f64 / 1000.0)
                } else {
                    0.0
                }
            });

        let prompt_eval_ms = parsed
            .pointer("/timings/prompt_ms")
            .and_then(Value::as_f64)
            .map(|value| value.round() as u64);

        let generation_ms = parsed
            .pointer("/timings/predicted_ms")
            .and_then(Value::as_f64)
            .map(|value| value.round() as u64);

        Ok(ProductionInferenceResult {
            ai_output,
            latency_ms,
            input_tokens,
            output_tokens,
            tokens_per_second,
            prompt_eval_ms,
            generation_ms,
            max_tokens,
        })
    }
}
