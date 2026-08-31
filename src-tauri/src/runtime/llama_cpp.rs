use crate::core::{
    capacity::CapacityTaskResult,
    certification_runner::{CertificationExecutor, ExecutionBatchResult},
    certification_workload::CertificationWorkload,
    generation_policy::{production_generation_settings_v1, GenerationSettings},
    model_registry::{capability_priority, model_spec, ModelSpecV1},
    production_prompt::compile_certification_prompt,
    workload_validator::validate_output,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct LlamaTaskExecution {
    pub result: CapacityTaskResult,
    pub output: String,
    pub validation_failures: Vec<String>,
    pub finish_reason: Option<String>,
    pub truncated: bool,
    pub attempts: u8,
    pub max_tokens_used: u32,
}

#[derive(Debug)]
struct LlamaAttempt {
    output: String,
    output_tokens: u32,
    runtime_tokens_per_second: f64,
    finish_reason: Option<String>,
    truncated: bool,
}

#[derive(Clone)]
pub struct LlamaCppHttpExecutor {
    client: Client,
    base_url: String,
}

fn model_spec_for_workload(
    workload: &CertificationWorkload,
) -> Result<&'static ModelSpecV1, String> {
    let priority = capability_priority(&workload.expected_required_model).ok_or_else(|| {
        format!(
            "unsupported_certification_capability:{}",
            workload.expected_required_model
        )
    })?;

    if priority.len() != 1 {
        return Err(format!(
            "ambiguous_certification_model:{}",
            workload.expected_required_model
        ));
    }

    model_spec(priority[0]).ok_or_else(|| format!("model_spec_missing:{}", priority[0]))
}

fn next_retry_budget(current: u32, allowed: u32) -> Option<u32> {
    if current >= allowed {
        return None;
    }

    let doubled = current.saturating_mul(2);
    let increased = current.saturating_add(64);

    Some(doubled.max(increased).min(allowed))
}

impl LlamaCppHttpExecutor {
    pub fn new(base_url: impl Into<String>) -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .no_proxy()
            .build()
            .map_err(|e| format!("llama_http_client_failed:{e}"))?;

        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }

    fn execute_attempt(
        &self,
        system_text: &str,
        user_text: &str,
        generation: &GenerationSettings,
        max_tokens: u32,
    ) -> Result<LlamaAttempt, String> {
        let mut body = json!({
            "model": "local-model",
            "messages": [
                {
                    "role": "system",
                    "content": system_text
                },
                {
                    "role": "user",
                    "content": user_text
                }
            ],
            "temperature": generation.temperature,
            "top_p": generation.top_p,
            "max_tokens": max_tokens,
            "stop": generation.stop,
            "stream": false
        });

        // JSON_CONSTRAINED_GENERATION_V1
        // Use llama.cpp grammar-constrained JSON generation whenever the
        // shared production generation policy classifies the task as JSON.
        if generation.mode == "json" {
            body["json_schema"] = json!({
                "type": "object"
            });
        }

        let started = Instant::now();

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", "Bearer no-key")
            .json(&body)
            .send()
            .map_err(|e| format!("llama_request_failed:{e}"))?;

        let status = response.status();

        let raw = response
            .text()
            .map_err(|e| format!("llama_response_read_failed:{e}"))?;

        let attempt_wall_time_ms = started.elapsed().as_millis() as u64;

        if !status.is_success() {
            let preview: String = raw.chars().take(500).collect();

            return Err(format!("llama_http_error:{}:{}", status.as_u16(), preview));
        }

        let parsed: Value =
            serde_json::from_str(&raw).map_err(|e| format!("llama_response_json_failed:{e}"))?;

        let output = parsed
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        let finish_reason = parsed
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(str::to_string);

        let output_tokens = parsed
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .or_else(|| {
                parsed
                    .pointer("/timings/predicted_n")
                    .and_then(Value::as_u64)
            })
            .unwrap_or(0) as u32;

        let runtime_tokens_per_second = parsed
            .pointer("/timings/predicted_per_second")
            .and_then(Value::as_f64)
            .unwrap_or_else(|| {
                if attempt_wall_time_ms > 0 {
                    output_tokens as f64 / (attempt_wall_time_ms as f64 / 1000.0)
                } else {
                    0.0
                }
            });

        let truncated = finish_reason.as_deref() == Some("length")
            || (finish_reason.is_none() && output_tokens >= max_tokens);

        Ok(LlamaAttempt {
            output,
            output_tokens,
            runtime_tokens_per_second,
            finish_reason,
            truncated,
        })
    }

    pub fn execute_one(
        &self,
        workload: &CertificationWorkload,
    ) -> Result<LlamaTaskExecution, String> {
        let compiled = compile_certification_prompt(workload)?;

        let spec = model_spec_for_workload(workload)?;

        let generation = production_generation_settings_v1(
            &compiled.user_text,
            None,
            Some(workload.max_output_tokens),
        );

        let allowed_max_tokens = spec.allowed_max_tokens.max(1);

        let mut max_tokens = generation.max_tokens.min(allowed_max_tokens).max(1);

        let total_started = Instant::now();

        let mut attempts = 1u8;

        let mut attempt = self.execute_attempt(
            &compiled.system_text,
            &compiled.user_text,
            &generation,
            max_tokens,
        )?;

        if attempt.truncated {
            if let Some(retry_budget) = next_retry_budget(max_tokens, allowed_max_tokens) {
                max_tokens = retry_budget;
                attempts = 2;

                attempt = self.execute_attempt(
                    &compiled.system_text,
                    &compiled.user_text,
                    &generation,
                    max_tokens,
                )?;
            }
        }

        let wall_time_ms = total_started.elapsed().as_millis() as u64;

        let validation = validate_output(workload, &attempt.output);

        let mut validation_failures = validation.failures;

        if attempt.truncated {
            validation_failures.push(format!("output_truncated:max_tokens={}", max_tokens));
        }

        let success = !attempt.output.is_empty() && !attempt.truncated;

        let output_valid = validation.valid && !attempt.truncated;

        let tokens_per_second = if attempts > 1 && wall_time_ms > 0 {
            attempt.output_tokens as f64 / (wall_time_ms as f64 / 1000.0)
        } else {
            attempt.runtime_tokens_per_second
        };

        Ok(LlamaTaskExecution {
            result: CapacityTaskResult {
                workload_id: workload.id.clone(),
                profile: workload.profile.clone(),
                success,
                output_valid,
                wall_time_ms,
                first_token_ms: None,
                output_tokens: attempt.output_tokens,
                tokens_per_second,
            },
            output: attempt.output,
            validation_failures,
            finish_reason: attempt.finish_reason,
            truncated: attempt.truncated,
            attempts,
            max_tokens_used: max_tokens,
        })
    }
}

impl CertificationExecutor for LlamaCppHttpExecutor {
    fn execute(
        &mut self,
        workloads: &[CertificationWorkload],
        concurrency: u16,
    ) -> Result<ExecutionBatchResult, String> {
        if concurrency == 0 {
            return Err("concurrency_must_be_at_least_one".into());
        }

        println!("CERTIFICATION_CONCURRENCY_STARTED={concurrency}");
        let started = Instant::now();

        let mut task_results = Vec::with_capacity(workloads.len());

        for chunk in workloads.chunks(concurrency as usize) {
            let completed_before_chunk = task_results.len();
            let total_workloads = workloads.len();

            let mut results = std::thread::scope(|scope| {
                let mut handles = Vec::new();

                for workload in chunk {
                    let executor = self.clone();

                    handles.push(scope.spawn(move || executor.execute_one(workload)));
                }

                let mut completed = Vec::new();

                for handle in handles {
                    match handle.join() {
                        Ok(Ok(execution)) => {
                            completed.push(execution.result);
                            let done = completed_before_chunk + completed.len();
                            println!(
                                    "CERTIFICATION_WORKLOAD_PROGRESS={concurrency}|{done}|{total_workloads}"
                                );
                        }

                        Ok(Err(error)) => {
                            return Err(error);
                        }

                        Err(_) => {
                            return Err("llama_parallel_worker_panicked".into());
                        }
                    }
                }

                Ok::<Vec<CapacityTaskResult>, String>(completed)
            })?;

            task_results.append(&mut results);
        }

        Ok(ExecutionBatchResult {
            wall_time_ms: started.elapsed().as_millis() as u64,
            peak_memory_bytes: None,
            thermal_throttled: false,
            task_results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_budget_expands_but_never_exceeds_ceiling() {
        assert_eq!(next_retry_budget(256, 512), Some(512));

        assert_eq!(next_retry_budget(384, 1024), Some(768));

        assert_eq!(next_retry_budget(1024, 2048), Some(2048));

        assert_eq!(next_retry_budget(512, 512), None);
    }
}
