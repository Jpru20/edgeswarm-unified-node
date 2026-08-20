use crate::core::task_client::TaskEnvelope;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct DeterministicResult {
    pub ai_output: String,
    pub latency_ms: u64,
    pub model_id_used: &'static str,
}

fn plan(prompt: &str) -> Option<Value> {
    let rest = prompt.split_once("EXACT_EXTRACTION_PLAN_V1:")?.1;
    let start = rest.find('{')?;
    let text = &rest[start..];

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return serde_json::from_str(&text[..=index]).ok();
                }
            }
            _ => {}
        }
    }

    None
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

fn pick_plan_match(text: &str, pattern: &str, plan: &Value) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    let values: Vec<_> = re.find_iter(text).collect();

    if values.is_empty() {
        return None;
    }

    let rule = plan
        .get("selectionRule")
        .or_else(|| plan.get("selection_rule"))
        .and_then(Value::as_str)
        .unwrap_or("first_match")
        .to_lowercase();

    let picked = if rule == "last_match" || rule == "last" {
        values.last()
    } else if rule == "nearest_after_anchor" {
        let anchor = plan
            .get("anchorPhrase")
            .or_else(|| plan.get("anchor_phrase"))
            .and_then(Value::as_str)
            .unwrap_or("");

        if anchor.is_empty() {
            values.first()
        } else {
            let lower_text = text.to_lowercase();
            let lower_anchor = anchor.to_lowercase();

            lower_text
                .find(&lower_anchor)
                .and_then(|pos| {
                    let after = pos + lower_anchor.len();
                    values.iter().find(|m| m.start() >= after)
                })
                .or_else(|| values.first())
        }
    } else {
        values.first()
    };

    picked.map(|m| {
        m.as_str()
            .trim()
            .trim_end_matches(&['.', ','][..])
            .to_string()
    })
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

        let normalized_field = field.to_lowercase();

        let pattern = match normalized_field.as_str() {
            "email" => Some(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
            "url" => Some(r"https?://[^\s)>\]]+"),
            "phone" => Some(r"\+?\d[\d\s().-]{7,}\d"),
            "date" => Some(r"\b\d{4}-\d{2}-\d{2}\b"),
            "wallet" => Some(r"0x[a-fA-F0-9]{40}"),
            "version" => Some(r"\bv?\d+(?:\.\d+){1,3}\b"),
            "percentage" => Some(r"\b\d+(?:\.\d+)?%"),
            "ticker" => Some(r"\b[A-Z]{1,6}\b"),
            "number" => Some(r"-?\d+(?:,\d{3})*(?:\.\d+)?"),
            _ => None,
        };

        if let Some(pattern) = pattern {
            if let Some(mut value) = pick_plan_match(text, pattern, &p) {
                if normalized_field == "ticker" {
                    value = value.to_uppercase();
                }
                if normalized_field == "number" {
                    value = value.replace(',', "");
                }
                return json!({"response":value}).to_string();
            }
        }
    }

    let normalized_prompt = prompt
        .strip_prefix("prompt://")
        .unwrap_or(prompt);

    let customer_prompt = normalized_prompt
        .rsplit_once("USER:")
        .map(|(_, value)| value.trim())
        .unwrap_or(normalized_prompt);

    let lower = customer_prompt.to_lowercase();

    if lower.contains("amount") ||
       lower.contains("reward") ||
       lower.contains("bounty") ||
       lower.contains("swarm") ||
       lower.contains(" usd")
    {
        if let Some(v) = amount(customer_prompt) {
            return json!({"response":v}).to_string();
        }
    }

    if lower.contains("email") {
        if let Ok(re) = Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}") {
            if let Some(v) = re.find_iter(customer_prompt)
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
                if let Some(v) = re.captures(customer_prompt).and_then(|c| c.get(1)) {
                    return json!({"response":v.as_str().to_uppercase()}).to_string();
                }
            }
        }
    }

    for (needle, pattern) in [
        ("url", r"https?://[^\s)>\]]+"),
        ("phone", r"\+?\d[\d\s().-]{7,}\d"),
        ("date", r"\b\d{4}-\d{2}-\d{2}\b"),
        ("wallet", r"0x[a-fA-F0-9]{40}"),
        ("version", r"\bv?\d+(?:\.\d+){1,3}\b"),
        ("percentage", r"\b\d+(?:\.\d+)?%"),
    ] {
        if lower.contains(needle) {
            if let Ok(re) = Regex::new(pattern) {
                if let Some(v) = re.find(customer_prompt) {
                    return json!({
                        "response":v.as_str()
                            .trim_end_matches(&['.', ','][..])
                    }).to_string();
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

fn explicit_matrix_compute(
    a: Vec<Vec<f64>>,
    b: Vec<Vec<f64>>,
) -> String {
    if a.is_empty() ||
       b.is_empty() ||
       a[0].is_empty() ||
       b[0].is_empty() ||
       a.iter().any(|row| row.len() != a[0].len()) ||
       b.iter().any(|row| row.len() != b[0].len()) ||
       a[0].len() != b.len() ||
       a.len() > 100 ||
       a[0].len() > 100 ||
       b.len() > 100 ||
       b[0].len() > 100
    {
        return json!({
            "error":"invalid_matrix_dimensions"
        }).to_string();
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
                result_row.push(
                    json!(
                        (total * 100000000.0).round() /
                        100000000.0
                    )
                );
            }
        }

        out.push(Value::Array(result_row));
    }

    json!({"response":out}).to_string()
}

fn infer_generated_matrix_size(prompt: &str) -> usize {
    let clean = prompt
        .strip_prefix("compute://")
        .unwrap_or(prompt)
        .trim();

    if let Ok(re) =
        Regex::new(r"(?i)\bsize\s*=\s*(\d{1,4})\b")
    {
        if let Some(size) = re
            .captures(clean)
            .and_then(|c| c.get(1))
            .and_then(|v| v.as_str().parse::<usize>().ok())
        {
            return size.clamp(1, 100);
        }
    }

    if let Ok(re) =
        Regex::new(r"(?i)\b(\d{1,4})\s*[x×]\s*(\d{1,4})\b")
    {
        if let Some(captures) = re.captures(clean) {
            let left = captures
                .get(1)
                .and_then(|v| v.as_str().parse::<usize>().ok());

            let right = captures
                .get(2)
                .and_then(|v| v.as_str().parse::<usize>().ok());

            if let (Some(left), Some(right)) = (left, right) {
                if left == right {
                    return left.clamp(1, 100);
                }
            }
        }
    }

    10
}

fn generated_matrix_compute(
    prompt: &str,
    checkpoint_indices: &[Value],
) -> String {
    let size = infer_generated_matrix_size(prompt);
    let total_cells = size * size;

    let seed_int: usize =
        size.to_string().bytes().map(usize::from).sum();

    let mut matrix_a = Vec::with_capacity(total_cells);
    let mut matrix_b = Vec::with_capacity(total_cells);

    for index in 0..total_cells {
        matrix_a.push(
            ((index + seed_int) % 1000) as f32 /
            1000.0_f32
        );

        matrix_b.push(
            ((index + seed_int + 999) % 1000) as f32 /
            1000.0_f32
        );
    }

    let mut result = vec![0.0_f32; total_cells];

    for row in 0..size {
        for col in 0..size {
            let mut total = 0.0_f32;

            for k in 0..size {
                let product =
                    matrix_a[row * size + k] *
                    matrix_b[k * size + col];

                total += product;
            }

            result[row * size + col] = total;
        }
    }

    let mut full_bytes =
        Vec::with_capacity(result.len() * 4);

    for value in &result {
        full_bytes.extend_from_slice(
            &value.to_le_bytes()
        );
    }

    let digest = Sha256::digest(&full_bytes);

    let result_hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let sample_len = result.len().min(1000);

    let mut sample_bytes =
        Vec::with_capacity(sample_len * 4);

    for value in result.iter().take(sample_len) {
        sample_bytes.extend_from_slice(
            &value.to_le_bytes()
        );
    }

    let sample_base64 =
        BASE64_STANDARD.encode(sample_bytes);

    let requested = checkpoint_indices
        .iter()
        .filter_map(|value| {
            value
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
        })
        .filter(|index| *index < result.len())
        .collect::<Vec<_>>();

    let fallback = [
        0usize,
        result.len() / 3,
        (result.len() * 2) / 3,
        result.len().saturating_sub(1),
    ];

    let mut selected = if requested.is_empty() {
        fallback
            .into_iter()
            .filter(|index| *index < result.len())
            .collect::<Vec<_>>()
    } else {
        requested
    };

    selected.sort_unstable();
    selected.dedup();

    let mut checkpoint_values =
        Map::<String, Value>::new();

    for index in selected {
        checkpoint_values.insert(
            index.to_string(),
            json!(result[index] as f64),
        );
    }

    json!({
        "type":"matrix_multiply",
        "size":size,
        "algorithmVersion":"1.0",
        "resultHash":result_hash,
        "sampleBase64":sample_base64,
        "checkpointValues":checkpoint_values
    }).to_string()
}

fn compute(task: &TaskEnvelope) -> String {
    let a = matrix(&task.prompt, "A");
    let b = matrix(&task.prompt, "B");

    match (a, b) {
        (Some(a), Some(b)) =>
            explicit_matrix_compute(a, b),

        (Some(_), None) =>
            json!({
                "error":"compute_failed",
                "message":"matrix B missing"
            }).to_string(),

        (None, Some(_)) =>
            json!({
                "error":"compute_failed",
                "message":"matrix A missing"
            }).to_string(),

        (None, None) =>
            generated_matrix_compute(
                &task.prompt,
                &task.checkpoint_indices,
            ),
    }
}

fn scrape(prompt: &str) -> String {
    let url = Regex::new(r"https?://[^\s)>\]]+").ok()
        .and_then(|r| r.find(prompt))
        .map(|m| m.as_str().trim_end_matches(&['.', ','][..]))
        .unwrap_or(prompt.trim());

    let result = Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("Mozilla/5.0 EdgeSwarmNode/", env!("CARGO_PKG_VERSION")))
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
            (compute(task), "deterministic-matrix-v1"),

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


#[cfg(test)]
mod exact_parity_tests {
    use super::*;

    #[test]
    fn nearest_after_anchor_selects_correct_email() {
        let prompt = r#"prompt://EXACT_EXTRACTION_PLAN_V1:
{"fieldType":"email","selectionRule":"nearest_after_anchor","anchorPhrase":"real contact","text":"backup@edgeswarm.io then real contact mac@edgeswarm.io"}"#;

        assert_eq!(
            exact(prompt),
            r#"{"response":"mac@edgeswarm.io"}"#
        );
    }

    #[test]
    fn structured_ticker_and_trailing_text_work() {
        let prompt = r#"EXACT_EXTRACTION_PLAN_V1:
{"fieldType":"ticker","selectionRule":"last_match","text":"NYSE: IBM then NASDAQ: NVDA"}
trailing text"#;

        assert_eq!(
            exact(prompt),
            r#"{"response":"NVDA"}"#
        );
    }

    #[test]
    fn amount_priority_ignores_level_number() {
        let prompt =
            "Level 1 node was rewarded 42.50 SWARM for this task.";

        assert_eq!(
            exact(prompt),
            r#"{"response":"42.50"}"#
        );
    }
}


#[cfg(test)]
mod compute_parity_tests {
    use super::*;

    #[test]
    fn generated_matrix_matches_existing_level1_contract() {
        let output = generated_matrix_compute(
            "compute://2x2",
            &[json!(0), json!(3)],
        );

        let parsed: Value =
            serde_json::from_str(&output).unwrap();

        assert_eq!(
            parsed["type"],
            "matrix_multiply"
        );

        assert_eq!(
            parsed["algorithmVersion"],
            "1.0"
        );

        assert_eq!(
            parsed["size"],
            2
        );

        assert_eq!(
            parsed["resultHash"],
            "b397731ca5c8e75ef1cf14e66c78ad817a116927f1da14126d5261955235838a"
        );

        assert_eq!(
            parsed["sampleBase64"],
            "3IKlOxzSqDuUEKw7YoGvOw=="
        );

        let first = parsed["checkpointValues"]["0"]
            .as_f64()
            .unwrap();

        let last = parsed["checkpointValues"]["3"]
            .as_f64()
            .unwrap();

        assert!(
            (first - 0.005051000043749809).abs() <
            0.00000001
        );

        assert!(
            (last - 0.005355999805033207).abs() <
            0.00000001
        );
    }

    #[test]
    fn generated_matrix_clamps_size_to_level1_limit() {
        assert_eq!(
            infer_generated_matrix_size(
                "compute://size=250"
            ),
            100
        );

        assert_eq!(
            infer_generated_matrix_size(
                "compute://25x25"
            ),
            25
        );
    }
}
