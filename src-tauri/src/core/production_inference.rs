use crate::core::{
    generation_policy::{
        production_generation_settings_adaptive_v1,
        production_generation_settings_v1,
    },
    vision_asset_fetch_v1::PreparedVisionAssetV1,
};
use base64::{
    engine::general_purpose::STANDARD as BASE64_STANDARD,
    Engine as _,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeContextV176 {
    pub generation_n_ctx: Option<u64>,
    pub runtime_n_ctx: Option<u64>,
    pub model_n_ctx_train: Option<u64>,
    pub total_slots: Option<u64>,
}

impl RuntimeContextV176 {
    fn from_props(v: &Value) -> Self {
        let read = |path: &str| v.pointer(path)
            .and_then(Value::as_u64).filter(|n| *n > 0);
        Self {
            generation_n_ctx: read("/default_generation_settings/n_ctx"),
            runtime_n_ctx: read("/n_ctx"),
            model_n_ctx_train: read("/n_ctx_train"),
            total_slots: read("/total_slots"),
        }
    }
}

#[derive(Debug, Default)]
pub struct RuntimeTelemetryV176 {
    pub context: RuntimeContextV176,
    pub prompt_n: Option<u64>,
    pub cache_n: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub first_content_ms: Option<u64>,
}

impl RuntimeTelemetryV176 {
    fn observe(&mut self, frame: &Value) {
        for (field, path) in [
            (&mut self.prompt_n, "/timings/prompt_n"),
            (&mut self.cache_n, "/timings/cache_n"),
            (&mut self.cached_tokens, "/usage/prompt_tokens_details/cached_tokens"),
        ] {
            if let Some(n) = frame.pointer(path).and_then(Value::as_u64) {
                *field = Some(n);
            }
        }
    }
}

#[derive(Debug)]
pub struct ProductionInferenceResult {
    pub runtime_telemetry_v176: RuntimeTelemetryV176,
    pub ai_output: String,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tokens_per_second: f64,
    pub prompt_eval_ms: Option<u64>,
    pub generation_ms: Option<u64>,
    pub max_tokens: u32,
    pub finish_reason: Option<String>,
    pub token_count_method: &'static str,
}

// Completion state is provider-reported; do not infer semantic completion from length.
fn completion_finish_reason_v1(message: &Value) -> Option<String> {
    message
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|reason| !reason.is_empty() && reason.len() <= 64)
        .map(str::to_owned)
}

// SWARM_LOCAL_MULTIMODAL_INFERENCE_V1
fn vision_chat_content_v171(
    prompt: &str,
    assets: &[PreparedVisionAssetV1],
) -> Result<Value, String> {
    if assets.is_empty() {
        return Err(
            "vision_inference_assets_empty"
                .into()
        );
    }

    if assets.len() > 4 {
        return Err(
            "vision_inference_asset_count_exceeded"
                .into()
        );
    }

    let total_bytes =
        assets
            .iter()
            .try_fold(
                0_u64,
                |total, asset| {
                    total
                        .checked_add(
                            asset.size_bytes
                        )
                        .ok_or_else(|| {
                            "vision_inference_total_size_overflow"
                                .to_string()
                        })
                },
            )?;

    if total_bytes >
        40 * 1024 * 1024
    {
        return Err(
            "vision_inference_total_size_exceeded"
                .into()
        );
    }

    let mut content =
        Vec::with_capacity(
            assets.len() + 1
        );

    content.push(json!({
        "type": "text",
        "text": prompt
    }));

    for asset in assets {
        if !matches!(
            asset.media_type.as_str(),
            "image/jpeg"
                | "image/png"
                | "image/webp"
        ) {
            return Err(
                "vision_inference_media_type_unsupported"
                    .into()
            );
        }

        let bytes =
            fs::read(
                &asset.path
            )
            .map_err(|_| {
                "vision_inference_local_asset_read_failed"
                    .to_string()
            })?;

        if bytes.len() as u64
            != asset.size_bytes
        {
            return Err(
                "vision_inference_local_asset_size_mismatch"
                    .into()
            );
        }

        let encoded =
            BASE64_STANDARD.encode(
                &bytes
            );

        let data_uri =
            format!(
                "data:{};base64,{}",
                asset.media_type,
                encoded
            );

        content.push(json!({
            "type": "image_url",
            "image_url": {
                "url": data_uri
            }
        }));
    }

    Ok(
        Value::Array(content)
    )
}

#[derive(Clone)]
pub struct ProductionLlamaClient {
    http: Client,
    base_url: String,
    context_v176: std::sync::Arc<std::sync::OnceLock<RuntimeContextV176>>,
}

// ADAPTIVE_INFERENCE_TIMEOUT_V1
const LEGACY_INFERENCE_TIMEOUT_MS_V1: u64 = 105_000;

const ADAPTIVE_DEFAULT_ABSOLUTE_CEILING_MS_V1: u64 =
    900_000;

fn inference_request_timeout_v1(
    adaptive_execution: bool,
    absolute_ceiling_ms: Option<u64>,
) -> Duration {
    let timeout_ms =
        if adaptive_execution {
            absolute_ceiling_ms
                .unwrap_or(
                    ADAPTIVE_DEFAULT_ABSOLUTE_CEILING_MS_V1
                )
                .max(1)
        } else {
            LEGACY_INFERENCE_TIMEOUT_MS_V1
        };

    Duration::from_millis(timeout_ms)
}


fn select_generation_policy_v176(
    inference_prompt: &str,
    policy_prompt: Option<&str>,
    adaptive: bool,
    planner_json: bool,
    budget: Option<u32>,
) -> crate::core::generation_policy::GenerationSettings {
    if planner_json {
        return production_generation_settings_v1(
            "Return valid JSON only", None, budget
        );
    }

    let policy = policy_prompt.unwrap_or(inference_prompt);

    if adaptive {
        production_generation_settings_adaptive_v1(policy,None,budget)
    } else {
        production_generation_settings_v1(policy,None,budget)
    }
}

impl ProductionLlamaClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, String> {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .no_proxy()
            .build()
            .map_err(|_| "production_llama_client_build_failed".to_string())?;

        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            context_v176: Default::default(),
        })
    }

    pub fn health_check(&self) -> Result<(), String> {
        let response = self
            .http
            .get(format!("{}/health", self.base_url))
            .timeout(Duration::from_secs(20))
            .send()
            .map_err(|_| "production_llama_health_request_failed".to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "production_llama_not_ready_http_{}",
                response.status().as_u16()
            ));
        }

        if crate::core::swarm_token_context_v1::is_loopback_runtime_v176(&self.base_url) {
            self.context_v176.get_or_init(|| {
                let context = self.http
                    .get(format!("{}/props", self.base_url))
                    .timeout(Duration::from_secs(3))
                    .send().ok()
                    .and_then(|r| r.error_for_status().ok())
                    .and_then(|r| r.json::<Value>().ok())
                    .map(|v| RuntimeContextV176::from_props(&v))
                    .unwrap_or_default();
                println!("RUNTIME_CONTEXT_V176={context:?}");
                context
            });
        }
        Ok(())
    }

    pub(crate) fn diagnostic_token_count_v176(&self, prompt: &str) -> Option<usize> {
        crate::core::swarm_token_context_v1::count_tokens_at_v176(
            &self.base_url, prompt
        ).ok()
    }

    // TRUE_NODE_CHUNK_STREAMING_V1
    // Reads llama.cpp OpenAI-compatible SSE deltas while preserving
    // the same complete final result used by normal consensus.
    pub fn execute_streaming<F>(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
        on_chunk: F,
    ) -> Result<ProductionInferenceResult, String>
    where
        F: FnMut(&str),
    {
        self.execute_streaming_with_policy(
            prompt,
            max_output_tokens,
            false,
            None,
            on_chunk,
            None,
            None,
        )
    }

    pub fn execute_streaming_adaptive<F>(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
        absolute_ceiling_ms: Option<u64>,
        on_chunk: F,
    ) -> Result<ProductionInferenceResult, String>
    where
        F: FnMut(&str),
    {
        self.execute_streaming_with_policy(
            prompt,
            max_output_tokens,
            true,
            absolute_ceiling_ms,
            on_chunk,
            None,
            None,
        )
    }

    fn execute_streaming_with_policy<F>(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
        adaptive_execution: bool,
        absolute_ceiling_ms: Option<u64>,
        mut on_chunk: F,
        policy_prompt: Option<&str>,
        funding_v176: Option<&crate::core::dynamic_funding_v176::FundingSessionV176>,
    ) -> Result<ProductionInferenceResult, String>
    where
        F: FnMut(&str),
    {
        let prompt = prompt.trim();

        if prompt.is_empty() {
            return Err("production_task_prompt_empty".into());
        }
        // No hidden task-length ceiling. A supplied budget remains authoritative;
        // otherwise the llama.cpp runtime stops naturally at EOS or context limit.
        let requested_u64 = max_output_tokens.unwrap_or(u32::MAX as u64).max(1);

        let requested =
            u32::try_from(requested_u64)
                .unwrap_or(u32::MAX);

        let generation = select_generation_policy_v176(
            prompt, policy_prompt, adaptive_execution, false,
            max_output_tokens.map(|_| requested),
        );

        let request_max_tokens: i64 =
            if generation.max_tokens == u32::MAX
            {
                -1
            } else {
                generation.max_tokens.max(1) as i64
            };

        let max_tokens =
            if request_max_tokens < 0 {
                0
            } else {
                request_max_tokens as u32
            };

        println!(
            "INFERENCE_OUTPUT_TOKEN_LIMIT={}",
            if request_max_tokens < 0 {
                "unbounded".to_string()
            } else {
                request_max_tokens.to_string()
            }
        );

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
            "max_tokens": request_max_tokens,
            "cache_prompt": !cfg!(target_os = "macos"),
            "stop": generation.stop,
            "stream": true,
            "stream_options": {
                "include_usage": true
            }
        });

        if let Some(funding) = funding_v176 {
            if request_max_tokens != -1 || !adaptive_execution {
                return Err("funding_inference_policy_conflict".into());
            }
            funding.check()?;
            body["swarmFundingV176"] = funding.request();
        }

        if adaptive_execution && generation.mode == "json" {
            body["json_schema"] = json!({
                "type": "object"
            });
        }

        let started = Instant::now();

        let request_timeout_v1 =
            inference_request_timeout_v1(
                adaptive_execution,
                absolute_ceiling_ms,
            );

        println!(
            "INFERENCE_REQUEST_TIMEOUT_MS={}",
            request_timeout_v1.as_millis()
        );

        let response = self
            .http
            .post(format!(
                "{}/v1/chat/completions",
                self.base_url
            ))
            .header("Authorization", "Bearer no-key")
            .json(&body)
            .timeout(request_timeout_v1)
            .send()
            .map_err(|error| {
                if error.is_timeout() {
                    "production_llama_stream_timeout".to_string()
                } else {
                    "production_llama_stream_request_failed".to_string()
                }
            })?;

        let status = response.status();

        if !status.is_success() {
            let body =
                response
                    .text()
                    .unwrap_or_default()
                    .replace('\r', " ")
                    .replace('\n', " ");

            let body: String =
                body
                    .chars()
                    .take(800)
                    .collect();

            return Err(runtime_http_failure_v176(status.as_u16(), &body));
        }let reader = BufReader::new(response);

        let mut output = String::new();

        let mut input_tokens = 0_u64;
        let mut output_tokens = 0_u64;
        let mut runtime_tps = None::<f64>;
        let mut telemetry = RuntimeTelemetryV176 {
            context: self.context_v176.get().copied().unwrap_or_default(),
            ..Default::default()
        };
        let mut prompt_eval_ms = None::<u64>;
        let mut generation_ms = None::<u64>;
        let mut finish_reason = None::<String>;
        let mut runtime_token_count_reported = false;

        let mut stream_done_v176 = false;
        for line_result in reader.lines() {
            if let Some(funding) = funding_v176 { funding.check()?; }
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
                stream_done_v176 = true;
                break;
            }

            let parsed: Value = serde_json::from_str(data)
                .map_err(|_| "production_llama_stream_json_failed".to_string())?;

            if funding_v176.is_some() && parsed.get("error").is_some() {
                return Err("funding_runtime_stream_error".into());
            }
            telemetry.observe(&parsed);
            if let Some(reason) = completion_finish_reason_v1(&parsed) {
                finish_reason = Some(reason);
            }

            let delta = parsed
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
                .or_else(|| parsed.pointer("/choices/0/text").and_then(Value::as_str))
                .unwrap_or("");

            if !delta.is_empty() {
                if telemetry.first_content_ms.is_none() {
                    telemetry.first_content_ms = Some(started.elapsed().as_millis() as u64);
                }
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
                runtime_token_count_reported = true;
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

        if let Some(funding) = funding_v176 {
            funding.check()?;
            if !stream_done_v176 || !matches!(finish_reason.as_deref(), Some("stop" | "length")) {
                return Err("funding_runtime_stream_incomplete".into());
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
            runtime_telemetry_v176: telemetry,
            ai_output,
            latency_ms,
            input_tokens,
            output_tokens,
            tokens_per_second,
            prompt_eval_ms,
            generation_ms,
            max_tokens,
            finish_reason,
            token_count_method: if runtime_token_count_reported {
                "runtime-reported"
            } else {
                "node-char-estimate"
            },
        })
    }

    pub fn execute(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
    ) -> Result<ProductionInferenceResult, String> {
        self.execute_with_policy(
            prompt,
            max_output_tokens,
            false,
            None,
            None,
            false,
            None,
        )
    }

    pub fn execute_vision(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
        assets: &[PreparedVisionAssetV1],
    ) -> Result<ProductionInferenceResult, String> {
        self.execute_with_policy(
            prompt,
            max_output_tokens,
            false,
            None,
            Some(assets),
            false,
            None,
        )
    }

    pub fn execute_adaptive(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
        absolute_ceiling_ms: Option<u64>,
    ) -> Result<ProductionInferenceResult, String> {
        self.execute_with_policy(
            prompt,
            max_output_tokens,
            true,
            absolute_ceiling_ms,
            None,
            false,
            None,
        )
    }

    pub(crate) fn execute_web_planner_v176(
        &self, prompt: &str,
    ) -> Result<ProductionInferenceResult, String> {
        self.execute_with_policy(
            prompt, Some(96), false, None, None, true, None,
        )
    }


    pub(crate) fn funding_base_url_v176(&self) -> &str { &self.base_url }

    pub(crate) fn execute_customer_v176(
        &self,
        prompt: &str,
        policy_prompt: &str,
        budget: Option<u64>,
        adaptive: bool,
        ceiling: Option<u64>,
        vision: Option<&[PreparedVisionAssetV1]>,
        callback: Option<&mut dyn FnMut(&str)>,
        funding_v176: Option<&crate::core::dynamic_funding_v176::FundingSessionV176>,
    ) -> Result<ProductionInferenceResult,String> {
        if funding_v176.is_some() && (vision.is_some() || callback.is_none()) {
            return Err("funding_runtime_mode_unsupported".into());
        }
        if let Some(assets) = vision {
            self.execute_with_policy(
                prompt,budget,false,None,Some(assets),
                false,Some(policy_prompt)
            )
        } else if let Some(callback) = callback {
            self.execute_streaming_with_policy(
                prompt,budget,adaptive,ceiling,callback,
                Some(policy_prompt),funding_v176,
            )
        } else {
            self.execute_with_policy(
                prompt,budget,adaptive,ceiling,None,
                false,Some(policy_prompt)
            )
        }
    }

    fn execute_with_policy(
        &self,
        prompt: &str,
        max_output_tokens: Option<u64>,
        adaptive_execution: bool,
        absolute_ceiling_ms: Option<u64>,
        vision_assets:
            Option<&[PreparedVisionAssetV1]>,
        planner_json: bool,
        policy_prompt: Option<&str>,
    ) -> Result<ProductionInferenceResult, String> {
        let prompt = prompt.trim();

        if prompt.is_empty() {
            return Err("production_task_prompt_empty".into());
        }
        // No hidden task-length ceiling. A supplied budget remains authoritative;
        // otherwise the llama.cpp runtime stops naturally at EOS or context limit.
        let requested_u64 = max_output_tokens.unwrap_or(u32::MAX as u64).max(1);

        let requested =
            u32::try_from(requested_u64)
                .unwrap_or(u32::MAX);

        let generation = select_generation_policy_v176(
            prompt, policy_prompt, adaptive_execution, planner_json,
            max_output_tokens.map(|_| requested),
        );

        let request_max_tokens: i64 =
            if generation.max_tokens == u32::MAX
            {
                -1
            } else {
                generation.max_tokens.max(1) as i64
            };

        let max_tokens =
            if request_max_tokens < 0 {
                0
            } else {
                request_max_tokens as u32
            };

        println!(
            "INFERENCE_OUTPUT_TOKEN_LIMIT={}",
            if request_max_tokens < 0 {
                "unbounded".to_string()
            } else {
                request_max_tokens.to_string()
            }
        );

        let user_content_v171 =
            match vision_assets {
                Some(assets) => {
                    println!(
                        "VISION_INFERENCE_ASSET_COUNT={}",
                        assets.len()
                    );

                    vision_chat_content_v171(
                        prompt,
                        assets,
                    )?
                }

                None =>
                    Value::String(
                        prompt.to_string()
                    ),
            };

        let mut body = json!({
            "model": "local-model",
            "messages": [
                {
                    "role": "user",
                    "content": user_content_v171
                }
            ],
            "temperature": generation.temperature,
            "top_p": generation.top_p,
            "max_tokens": request_max_tokens,
            "cache_prompt": !cfg!(target_os = "macos"),
            "stop": generation.stop,
            "stream": false
        });

        // JSON_CONSTRAINED_GENERATION_V1
        // Keep live customer execution aligned with certification:
        // JSON-mode tasks use llama.cpp constrained JSON generation.
        // WEB_PLANNER_THINKING_ISOLATION_V176
        if planner_json {
            body["chat_template_kwargs"] = json!({"enable_thinking": false});
            body["json_schema"] = json!({
    "oneOf": [
        {
            "type": "object",
            "properties": {
                "action": {
                    "const": "search"
                },
                "query": {
                    "type": "string"
                }
            },
            "required": [
                "action",
                "query"
            ],
            "additionalProperties": false
        },
        {
            "type": "object",
            "properties": {
                "action": {
                    "const": "done"
                }
            },
            "required": [
                "action"
            ],
            "additionalProperties": false
        }
    ]
});
        } else if generation.mode == "json" {
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
            .timeout(
                inference_request_timeout_v1(
                    adaptive_execution,
                    absolute_ceiling_ms,
                )
            )
            .send()
            .map_err(|error| {
                if error.is_timeout() {
                    "production_llama_timeout".to_string()
                } else {
                    "production_llama_request_failed".to_string()
                }
            })?;

        let status = response.status();

        let raw = response
            .text()
            .map_err(|_| "production_llama_response_read_failed".to_string())?;

        if !status.is_success() {
            let body =
                raw
                    .replace('\r', " ")
                    .replace('\n', " ");

            let body: String =
                body
                    .chars()
                    .take(800)
                    .collect();

            return Err(runtime_http_failure_v176(status.as_u16(), &body));
        }let parsed: Value = serde_json::from_str(&raw)
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

        let mut telemetry = RuntimeTelemetryV176 {
            context: self.context_v176.get().copied().unwrap_or_default(),
            ..Default::default()
        };
        telemetry.observe(&parsed);
        let finish_reason = completion_finish_reason_v1(&parsed);

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
            runtime_telemetry_v176: telemetry,
            ai_output,
            latency_ms,
            input_tokens,
            output_tokens,
            tokens_per_second,
            prompt_eval_ms,
            generation_ms,
            max_tokens,
            finish_reason,
            token_count_method: if output_tokens > 0 {
                "runtime-reported"
            } else {
                "node-unavailable"
            },
        })
    }
}

#[cfg(test)]
mod vision_payload_tests_v171 {
    use super::*;

    #[test]
    fn vision_payload_v171_uses_data_uri_not_file_path() {
        let root =
            std::env::temp_dir()
                .join(
                    "swarm-vision-payload-v171"
                );

        let _ =
            std::fs::remove_dir_all(
                &root
            );

        std::fs::create_dir_all(
            &root
        )
        .unwrap();

        let path =
            root.join(
                "image.png"
            );

        std::fs::write(
            &path,
            b"png-test"
        )
        .unwrap();

        let asset =
            PreparedVisionAssetV1 {
                asset_id:
                    "img_test".into(),
                media_type:
                    "image/png".into(),
                sha256:
                    "a".repeat(64),
                size_bytes:
                    8,
                path,
            };

        let content =
            vision_chat_content_v171(
                "Describe this image.",
                &[asset],
            )
            .unwrap();

        assert_eq!(
            content[0]["type"],
            "text"
        );

        let url =
            content[1]
                ["image_url"]
                ["url"]
                .as_str()
                .unwrap();

        assert!(
            url.starts_with(
                "data:image/png;base64,"
            )
        );

        assert!(
            !url.contains(
                "swarm-vision-payload-v171"
            )
        );

        let _ =
            std::fs::remove_dir_all(
                root
            );
    }
}

// Completion/length tests reuse the production parser; no synthetic executor.
#[cfg(test)]
mod finish_reason_tests {
    use super::*;
    #[test]
    fn records_natural_stop_and_token_exhaustion() {
        assert_eq!(
            completion_finish_reason_v1(&json!({"choices":[{"finish_reason":"stop"}]})),
            Some("stop".to_string())
        );
        assert_eq!(
            completion_finish_reason_v1(&json!({"choices":[{"finish_reason":"length"}]})),
            Some("length".to_string())
        );
    }
    #[test]
    fn missing_finish_reason_stays_unknown() {
        assert_eq!(completion_finish_reason_v1(&json!({"choices":[{}]})), None);
        assert_eq!(completion_finish_reason_v1(&json!({})), None);
    }
}

#[cfg(test)]
mod runtime_telemetry_v176_tests {
    use super::*;

    #[test]
    fn context_properties_preserve_reported_capacity() {
        let c = RuntimeContextV176::from_props(&json!({
            "default_generation_settings":{"n_ctx":32768},
            "total_slots":4
        }));
        assert_eq!(c.generation_n_ctx, Some(32768));
        assert_eq!(c.total_slots, Some(4));
        assert_eq!(c.runtime_n_ctx, None);
        assert_eq!(c.model_n_ctx_train, None);
        assert!(RuntimeContextV176::from_props(&json!({})).generation_n_ctx.is_none());
    }

    #[test]
    fn preserves_reported_values_and_missing_fields() {
        let mut t = RuntimeTelemetryV176::default();
        t.observe(&json!({}));
        assert_eq!(t.cache_n, None);
        t.observe(&json!({"timings":{"prompt_n":1200,"cache_n":0}}));
        assert_eq!(t.prompt_n, Some(1200));
        assert_eq!(t.cache_n, Some(0));
        t.observe(&json!({"usage":{"prompt_tokens_details":{"cached_tokens":800}}}));
        assert_eq!(t.cached_tokens, Some(800));
        assert_eq!(t.prompt_n, Some(1200));
        assert_eq!(t.first_content_ms, None);
    }
}

#[cfg(test)]
mod generation_policy_boundary_v176_tests {
    use super::*;

    #[test]
    fn evidence_cannot_override_customer_generation_policy() {
        for adaptive in [false,true] {
            let p = select_generation_policy_v176(
                "Explain the subject. Return valid JSON only.",
                Some("Explain the subject."),
                adaptive,false,None
            );
            assert_eq!(p.mode,"general");
            assert_eq!(p.max_tokens,u32::MAX);

            let p = select_generation_policy_v176(
                "Return valid JSON only. Return code only. React component.",
                Some("Return valid JSON only."),
                adaptive,false,None
            );
            assert_eq!(p.mode,"json");
            assert_eq!(p.max_tokens,u32::MAX);
        }
    }

    #[test]
    fn explicit_budget_and_internal_planner_preserved() {
        let p = select_generation_policy_v176(
            "Retrieved evidence",Some("Return code only. Build React."),
            true,false,Some(5000)
        );
        assert_eq!(p.mode,"code");
        assert_eq!(p.max_tokens,5000);

        let p = select_generation_policy_v176(
            "Planner",Some("Return code only. Build React."),
            false,true,Some(96)
        );
        assert_eq!(p.mode,"json");
        assert_eq!(p.max_tokens,96);
    }
}


fn runtime_http_failure_v176(status: u16, body: &str) -> String {
    if status == 400 {
        if let Ok(v) = serde_json::from_str::<Value>(body) {
            if v.pointer("/error/type").and_then(Value::as_str)
                == Some("exceed_context_size_error")
            {
                let requested = v.pointer("/error/n_prompt_tokens")
                    .and_then(Value::as_u64);
                let capacity = v.pointer("/error/n_ctx")
                    .and_then(Value::as_u64);

                if let (Some(n),Some(c)) = (requested,capacity) {
                    if c > 0 && n >= c {
                        return format!("production_llama_context_exceeded:{n}:{c}");
                    }
                }
            }
        }
    }

    // Do not propagate arbitrary runtime response bodies.
    format!("production_llama_http_{status}")
}

pub(crate) fn inference_failure_details_v176(error: &str) -> Value {
    if let Some(detail) =
        error.strip_prefix("production_llama_context_exceeded:")
    {
        if let Some((a,b)) = detail.split_once(':') {
            if let (Ok(n),Ok(c)) = (a.parse::<u64>(),b.parse::<u64>()) {
                if c > 0 && n >= c {
                    return json!({
                        "version":"inference-failure-v176",
                        "code":"context_capacity_exceeded",
                        "stage":"prompt_admission",
                        "httpStatus":400,
                        "promptTokens":n,
                        "contextTokens":c,
                        "excessTokens":n-c
                    });
                }
            }
        }
    }

    if let Some(status) = error.strip_prefix("production_llama_http_")
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|s| (100..=599).contains(s))
    {
        return json!({
            "version":"inference-failure-v176",
            "code":"runtime_http_error",
            "httpStatus":status
        });
    }

    let code = match error {
        "dynamic_funding_reload_required" => "dynamic_funding_reload_required",
        "dynamic_funding_execution_deadline" => "dynamic_funding_execution_deadline",
        "dynamic_funding_progress_fenced" => "dynamic_funding_progress_fenced",
        "funding_start_unconfirmed" => "funding_service_unconfirmed",
        "funding_runtime_stream_incomplete" => "funding_runtime_stream_incomplete",
        "funding_runtime_stream_error" => "funding_runtime_stream_error",
        "production_llama_timeout" |
        "production_llama_stream_timeout" => "runtime_timeout",
        "production_llama_request_failed" |
        "production_llama_stream_request_failed" => "runtime_request_failed",
        _ => "runtime_inference_failed"
    };

    json!({
        "version":"inference-failure-v176",
        "code":code
    })
}

#[cfg(test)]
mod runtime_failure_v176_tests {
    use super::*;

    #[test]
    fn preserves_measured_context_failure() {
        let raw = r#"{"error":{"code":400,"message":"request exceeds context","type":"exceed_context_size_error","n_prompt_tokens":2077,"n_ctx":1024}}"#;
        let failure = runtime_http_failure_v176(400,raw);
        let d = inference_failure_details_v176(&failure);

        assert_eq!(d["code"],"context_capacity_exceeded");
        assert_eq!(d["promptTokens"],2077);
        assert_eq!(d["contextTokens"],1024);
        assert_eq!(d["excessTokens"],1053);
    }

    #[test]
    fn unknown_errors_do_not_leak_runtime_body() {
        let failure = runtime_http_failure_v176(
            500,"PRIVATE_CUSTOMER_CONTENT"
        );
        let d = inference_failure_details_v176(&failure);
        assert_eq!(d["code"],"runtime_http_error");
        assert_eq!(d["httpStatus"],500);
        assert!(!d.to_string().contains("PRIVATE_CUSTOMER_CONTENT"));
    }

    #[test]
    fn inconsistent_capacity_is_not_fabricated() {
        let raw = r#"{"error":{"type":"exceed_context_size_error","n_prompt_tokens":50,"n_ctx":1024}}"#;
        let failure = runtime_http_failure_v176(400,raw);
        let d = inference_failure_details_v176(&failure);
        assert_eq!(d["code"],"runtime_http_error");
        assert!(d.get("excessTokens").is_none());
    }
}
