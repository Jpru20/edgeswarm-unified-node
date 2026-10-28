use reqwest::blocking::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub struct EvidenceSource<'a> {
    pub id: &'a str,
    pub text: &'a str,
    pub source_hash: &'a str,
}

pub struct ProtectedSpan<'a> {
    pub source_id: &'a str,
    pub start: usize,
    pub end: usize,
    pub quote_hash: &'a str,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// Only central-plan-protected spans may be selected.
pub fn pack_locked_context(
    prompt: &str,
    sources: &[EvidenceSource<'_>],
    locks: &[ProtectedSpan<'_>],
) -> Result<String, String> {
    if prompt.is_empty() || locks.is_empty() || sources.len() > 32 || locks.len() > 128 {
        return Err("protected_context_contract_invalid".into());
    }

    let mut output = prompt.to_owned();
    output.push_str("\n[SWARM_SOURCE_PASSAGES]\n");

    for lock in locks {
        let matches: Vec<_> = sources.iter().filter(|s| s.id == lock.source_id).collect();
        if matches.len() != 1 {
            return Err("protected_source_missing_or_ambiguous".into());
        }
        let source = matches[0];
        if digest(source.text.as_bytes()) != source.source_hash {
            return Err("source_hash_mismatch".into());
        }
        let quote = source
            .text
            .get(lock.start..lock.end)
            .filter(|q| !q.is_empty())
            .ok_or("protected_range_invalid")?;
        if digest(quote.as_bytes()) != lock.quote_hash {
            return Err("protected_quote_hash_mismatch".into());
        }
        output.push_str(&format!(
            "[{}:{}-{}] {}\n",
            source.id, lock.start, lock.end, quote
        ));
    }

    output.push_str("[/SWARM_SOURCE_PASSAGES]");
    Ok(output)
}

// Diagnostics only: count tokens with the local model tokenizer.
pub fn count_local_tokens(prompt: &str) -> Result<usize, String> {
    let http = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| "token_client_failed")?;

    let template: Value = http
        .post("http://127.0.0.1:18081/apply-template")
        .json(&json!({"messages":[{"role":"user","content":prompt}]}))
        .send()
        .map_err(|_| "template_unavailable")?
        .error_for_status()
        .map_err(|_| "template_http_error")?
        .json()
        .map_err(|_| "template_invalid")?;

    let rendered = template["prompt"]
        .as_str()
        .ok_or("template_missing_prompt")?;

    let counted: Value = http
        .post("http://127.0.0.1:18081/tokenize")
        .json(&json!({
            "content":rendered,
            "parse_special":true,
            "add_special":false
        }))
        .send()
        .map_err(|_| "tokenizer_unavailable")?
        .error_for_status()
        .map_err(|_| "tokenizer_http_error")?
        .json()
        .map_err(|_| "tokenizer_invalid")?;

    counted["tokens"]
        .as_array()
        .map(|x| x.len())
        .ok_or_else(|| "tokenizer_missing_tokens".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_quotes_preserved() {
        let source = "Revenue $248M. Irrelevant footer.";
        let quote = "Revenue $248M.";
        let source_hash = digest(source.as_bytes());
        let quote_hash = digest(quote.as_bytes());
        let sources = [EvidenceSource {
            id: "a",
            text: source,
            source_hash: &source_hash,
        }];
        let locks = [ProtectedSpan {
            source_id: "a",
            start: 0,
            end: quote.len(),
            quote_hash: &quote_hash,
        }];
        let packed = pack_locked_context("Compare Q3", &sources, &locks).unwrap();
        assert!(packed.contains(quote));
        assert!(!packed.contains("footer"));
    }

    #[test]
    fn missing_protection_rejected() {
        assert!(pack_locked_context("Question", &[], &[]).is_err());
    }
}
