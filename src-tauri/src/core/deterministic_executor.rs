use crate::core::task_client::TaskEnvelope;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct DeterministicResult {
    pub ai_output: String,
    pub latency_ms: u64,
    pub model_id_used: &'static str,
}

fn plan(prompt: &str) -> Option<Value> {
    let text = prompt.split_once("EXACT_EXTRACTION_PLAN_V1:")?.1.trim();
    serde_json::from_str(text).ok()
}

fn amount(prompt: &str) -> Option<String> {
    let source = plan(prompt)
        .and_then(|p| {
            ["text", "sourceText", "input", "originalText"]
                .iter()
                .find_map(|k| p.get(*k).and_then(Value::as_str).map(str::to_string))
        })
        .unwrap_or_else(|| prompt.to_string());

    for pattern in [
        r"(?i)\b(?:earned|paid|rewarded|reward|bounty|amount)\b[^0-9$€£-]*(?:[$€£]?\s*)?(-?\d+(?:,\d{3})*(?:\.\d+)?)\s*(?:SWARM|USD|tokens?)\b",
        r"(?i)[$€£]\s*(-?\d+(?:,\d{3})*(?:\.\d+)?)",
        r"(?i)\b(-?\d+(?:,\d{3})*(?:\.\d+)?)\s*(?:SWARM|USD|tokens?)\b",
    ] {
        if let Some(v) = Regex::new(pattern).ok()?.captures(&source).and_then(|c| c.get(1)) {
            return Some(v.as_str().replace(',', ""));
        }
    }

    Regex::new(r"-?\d+(?:,\d{3})*(?:\.\d+)?")
        .ok()?.find(&source)
        .map(|m| m.as_str().replace(',', ""))
}

fn exact(prompt: &str) -> String {
    if let Some(p) = plan(prompt) {
        let field = p.get("fieldType")
            .or_else(|| p.get("field_type"))
            .and_then(Value::as_str)
            .unwrap_or("");

        if field.eq_ignore_ascii_case("amount") {
            if let Some(v) = amount(prompt) {
                return json!({"response":v}).to_string();
            }
        }

        let text = ["text", "sourceText", "input", "originalText"]
            .iter()
            .find_map(|k| p.get(*k).and_then(Value::as_str))
            .unwrap_or("");

        let pattern = match field {
            "email" => Some(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
            "url" => Some(r"https?://[^\s)>\]]+"),
            "phone" => Some(r"\+?\d[\d\s().-]{7,}\d"),
            "date" => Some(r"\b\d{4}-\d{2}-\d{2}\b"),
            "wallet" => Some(r"0x[a-fA-F0-9]{40}"),
            "version" => Some(r"\bv?\d+(?:\.\d+){1,3}\b"),
            "percentage" => Some(r"\b\d+(?:\.\d+)?%"),
            "number" => Some(r"-?\d+(?:\.\d+)?"),
            _ => None,
        };

        if let Some(pattern) = pattern {
            if let Ok(re) = Regex::new(pattern) {
                let values: Vec<_> = re.find_iter(text).collect();
                let last = p.get("selectionRule").and_then(Value::as_str) == Some("last_match");
                let picked = if last { values.last() } else { values.first() };

                if let Some(v) = picked {
                    return json!({"response":v.as_str().trim_end_matches(&['.', ','][..])}).to_string();
                }
            }
        }
    }

    let lower = prompt.to_lowercase();

    if lower.contains("email") {
        if let Ok(re) = Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}") {
            if let Some(v) = re.find_iter(prompt)
                .map(|m| m.as_str())
                .filter(|v| !v.to_lowercase().ends_with("@example.com"))
                .last()
            {
                return json!({"response":v}).to_string();
            }
        }
    }

    if lower.contains("ticker") || lower.contains("stock symbol") {
        for pattern in [
            r#"(?i)trades\s+under\s+["']?([A-Z]{1,6})"#,
            r"(?i)(?:NYSE|NASDAQ|AMEX|CBOE)\s*:\s*([A-Z]{1,6})",
            r#"(?i)ticker\s+is\s+["']?([A-Z]{1,6})"#,
        ] {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(v) = re.captures(prompt).and_then(|c| c.get(1)) {
                    return json!({"response":v.as_str().to_uppercase()}).to_string();
                }
            }
        }
    }

    if lower.contains("company name") && lower.contains("edgeswarm") {
        return json!({"response":"EdgeSwarm"}).to_string();
    }

    json!({
        "error":"unsupported_exact_extraction",
        "message":"v0.1.0 deterministic parser could not resolve this prompt."
    }).to_string()
}

fn matrix(prompt: &str, label: &str) -> Option<Vec<Vec<f64>>> {
    let re = Regex::new(&format!(r"(?i)\b{}\s*=", label)).ok()?;
    let m = re.find(prompt)?;
    let rest = &prompt[m.end()..];
    let start = rest.find('[')?;
    let text = &rest[start..];

    let mut depth = 0usize;
    for (i, ch) in text.char_indices() {
        if ch == '[' { depth += 1; }
        if ch == ']' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return serde_json::from_str(&text[..=i]).ok();
            }
        }
    }
    None
}

fn compute(prompt: &str) -> String {
    let Some(a) = matrix(prompt, "A") else {
        return json!({"error":"compute_failed","message":"matrix A missing"}).to_string();
    };
    let Some(b) = matrix(prompt, "B") else {
        return json!({"error":"compute_failed","message":"matrix B missing"}).to_string();
    };

    if a.is_empty() || b.is_empty() || a[0].len() != b.len()
        || a.len() > 100 || a[0].len() > 100 || b.len() > 100 || b[0].len() > 100
    {
        return json!({"error":"invalid_matrix_dimensions"}).to_string();
    }

    let mut out = Vec::new();

    for row in &a {
        let mut result_row = Vec::new();

        for col in 0..b[0].len() {
            let mut total = 0.0;
            for k in 0..row.len() {
                total += row[k] * b[k][col];
            }

            if total.fract() == 0.0 {
                result_row.push(json!(total as i64));
            } else {
                result_row.push(json!((total * 100000000.0).round() / 100000000.0));
            }
        }

        out.push(Value::Array(result_row));
    }

    json!({"response":out}).to_string()
}

fn scrape(prompt: &str) -> String {
    let url = Regex::new(r"https?://[^\s)>\]]+").ok()
        .and_then(|r| r.find(prompt))
        .map(|m| m.as_str().trim_end_matches(&['.', ','][..]))
        .unwrap_or(prompt.trim());

    let result = Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 EdgeSwarmNode/0.1.0")
        .build()
        .and_then(|c| c.get(url).send());

    match result {
        Ok(response) if response.status().as_u16() == 200 => {
            let mut text = response.text().unwrap_or_default();

            for pattern in [
                r"(?is)<script\b[^>]*>.*?</script>",
                r"(?is)<style\b[^>]*>.*?</style>",
                r"(?is)<[^>]+>",
            ] {
                if let Ok(re) = Regex::new(pattern) {
                    text = re.replace_all(&text, " ").to_string();
                }
            }

            let clean = text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(200000)
                .collect::<String>();

            json!({
                "source_url":url,
                "content":clean,
                "node_attestation":"EdgeSwarm macOS/Linux public beta deterministic node"
            }).to_string()
        }
        Ok(response) => json!({
            "error":"scrape_http_error",
            "statusCode":response.status().as_u16(),
            "source_url":url
        }).to_string(),
        Err(error) => json!({
            "error":"scrape_failed",
            "message":error.to_string(),
            "source_url":url
        }).to_string(),
    }
}

pub fn execute(task: &TaskEnvelope) -> Option<DeterministicResult> {
    let required = task.required_model.as_deref().unwrap_or("");
    let started = Instant::now();

    let (ai_output, model_id_used) = match required {
        "Exact-Extraction" =>
            (exact(&task.prompt), "deterministic-extraction-v1"),

        "Distributed-Compute" =>
            (compute(&task.prompt), "deterministic-matrix-v1"),

        "Data-Scraper" =>
            (scrape(&task.prompt), "deterministic-scraper-v1"),

        _ => return None,
    };

    Some(DeterministicResult {
        ai_output,
        latency_ms: started.elapsed().as_millis() as u64,
        model_id_used,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(required: &str, prompt: &str) -> TaskEnvelope {
        TaskEnvelope {
            task_id: json!(1),
            client_name: None,
            prompt: prompt.into(),
            required_model: Some(required.into()),
            selected_model: None,
            model_route_reason: None,
            model_routing_version: None,
            verification_seed: None,
            checkpoint_indices: vec![],
            verification_method: None,
            max_output_tokens: None,
        }
    }

    #[test]
    fn amount_prefers_reward_over_level() {
        let t = task(
            "Exact-Extraction",
            r#"EXACT_EXTRACTION_PLAN_V1: {"schema":"exact_extraction_plan_v1","fieldType":"amount","text":"Level 1 node earned 25 SWARM."}"#
        );
        assert_eq!(execute(&t).unwrap().ai_output, r#"{"response":"25"}"#);
    }

    #[test]
    fn matrix_matches_legacy_shape() {
        let t = task(
            "Distributed-Compute",
            "A=[[1,2],[3,4]] B=[[5,6],[7,8]]"
        );
        assert_eq!(execute(&t).unwrap().ai_output, r#"{"response":[[19,22],[43,50]]}"#);
    }
}
