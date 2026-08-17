use edgeswarm_unified_node_lib::runtime::llama_process::{LlamaProcessConfig, ManagedLlamaProcess};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::{env, path::PathBuf, time::Duration};

fn main() -> Result<(), String> {
    let model_path = env::var_os("EDGESWARM_MODEL_PATH")
        .map(PathBuf::from)
        .ok_or_else(|| "EDGESWARM_MODEL_PATH_required".to_string())?;

    let config = LlamaProcessConfig::for_model(model_path)?;

    println!("LLAMA_EXECUTABLE={}", config.executable.display());
    println!("MODEL_PATH={}", config.model_path.display());
    println!("RUNTIME_ACCELERATION=cpu");

    let mut runtime = ManagedLlamaProcess::start(&config)?;

    println!("LLAMA_MODEL_READY=true");
    println!("LLAMA_BASE_URL={}", runtime.base_url());

    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .no_proxy()
        .build()
        .map_err(|e| format!("smoke_http_client_failed:{e}"))?;

    let body = json!({
        "model": "local-model",
        "messages": [
            {
                "role": "system",
                "content": "You are a concise assistant."
            },
            {
                "role": "user",
                "content": "A system processed 360 requests in 18 seconds. What was the average requests per second?"
            }
        ],
        "temperature": 0.0,
        "top_p": 0.1,
        "max_tokens": 96,
        "stream": false
    });

    let response = client
        .post(format!("{}/v1/chat/completions", runtime.base_url()))
        .header("Authorization", "Bearer no-key")
        .json(&body)
        .send()
        .map_err(|e| format!("smoke_inference_request_failed:{e}"))?;

    let status = response.status();

    let value = response
        .json::<Value>()
        .map_err(|e| format!("smoke_inference_json_failed:{e}"))?;

    if !status.is_success() {
        runtime.stop();
        return Err(format!("smoke_inference_http_{}:{value}", status.as_u16()));
    }

    let output = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();

    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .unwrap_or("");

    let completion_tokens = value
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    println!("LOCAL_RESPONSE={output}");
    println!("FINISH_REASON={finish_reason}");
    println!("COMPLETION_TOKENS={completion_tokens}");

    let pass = output.contains("20") && finish_reason == "stop";

    println!("RUST_RUNTIME_SMOKE_PASS={pass}");

    runtime.stop();
    println!("LLAMA_SERVER_STOPPED=true");

    if pass {
        Ok(())
    } else {
        Err("rust_runtime_smoke_failed".into())
    }
}
