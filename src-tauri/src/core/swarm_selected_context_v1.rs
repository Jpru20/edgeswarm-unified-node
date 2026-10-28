use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    ConversationUser,
    ConversationAssistant,
    PersistentMemory,
    WebEvidence,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedExcerpt {
    pub kind: ContextKind,
    pub id: String,
    pub text: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectedContextPacket {
    pub version: String,
    pub task_id: String,
    pub plan_id: String,
    pub excerpts: Vec<SelectedExcerpt>,
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

// Caller must authenticate the central execution contract.
// No source selection, disk writes, logging or caching occurs here.
pub fn assemble(packet_json: &str, expected_task_id: &str) -> Result<String, &'static str> {
    if packet_json.len() > 131072 {
        return Err("PACKET_TOO_LARGE");
    }

    let packet: SelectedContextPacket =
        serde_json::from_str(packet_json).map_err(|_| "PACKET_INVALID")?;

    if packet.version != "swarm-selected-context-v1"
        || packet.task_id != expected_task_id
        || !valid_id(&packet.plan_id)
        || packet.excerpts.len() > 64
    {
        return Err("CONTEXT_CONTRACT_INVALID");
    }

    if packet.excerpts.is_empty() {
        return Ok(String::new());
    }

    let mut seen = HashSet::new();
    let mut total = 0usize;
    let mut output = String::from("[SWARM_SELECTED_CONTEXT_V1]\n");

    for entry in &packet.excerpts {
        total = total.saturating_add(entry.text.len());

        if !valid_id(&entry.id)
            || !seen.insert(&entry.id)
            || entry.text.is_empty()
            || entry.text.len() > 8192
            || total > 65536
        {
            return Err("CONTEXT_LIMIT_INVALID");
        }

        if !hash(&entry.text).eq_ignore_ascii_case(&entry.sha256) {
            return Err("CONTEXT_HASH_MISMATCH");
        }

        let line = json!({
            "kind": &entry.kind,
            "id": &entry.id,
            "text": &entry.text
        });

        output.push_str(&line.to_string());
        output.push('\n');
    }

    output.push_str("[/SWARM_SELECTED_CONTEXT_V1]");
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        let text = "Enterprise pricing is subscription-based.";
        json!({
            "version": "swarm-selected-context-v1",
            "taskId": "task-101",
            "planId": "plan-101",
            "excerpts": [{
                "kind": "persistent_memory",
                "id": "m07",
                "text": text,
                "sha256": hash(text)
            }]
        })
        .to_string()
    }

    #[test]
    fn selected_memory_preserved() {
        let result = assemble(&fixture(), "task-101").unwrap();
        assert!(result.contains("subscription-based"));
        assert!(result.contains("m07"));
    }

    #[test]
    fn cross_task_context_rejected() {
        assert!(assemble(&fixture(), "task-999").is_err());
    }

    #[test]
    fn altered_memory_rejected() {
        let mut data: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
        data["excerpts"][0]["text"] = json!("Altered memory");
        assert!(assemble(&data.to_string(), "task-101").is_err());
    }

    #[test]
    fn full_history_field_rejected() {
        let mut data: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
        data["originalConversation"] = json!("Full history");
        assert!(assemble(&data.to_string(), "task-101").is_err());
    }

    #[test]
    fn empty_context_allowed() {
        let mut data: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
        data["excerpts"] = json!([]);
        assert_eq!(assemble(&data.to_string(), "task-101").unwrap(), "");
    }
}

pub fn prepare_inference_prompt_v1(
    prompt: &str,
    packet_json: Option<&str>,
    expected_task_id: &str,
    enabled: bool,
    retrieval_required: bool,
) -> Result<String, &'static str> {
    let Some(packet) = packet_json else {
        return Ok(prompt.to_owned());
    };
    if !enabled {
        return Err("CONTEXT_CANARY_DISABLED");
    }
    if retrieval_required {
        return Err("CONTEXT_RETRIEVAL_NOT_YET_SUPPORTED");
    }
    let selected = assemble(packet, expected_task_id)?;
    if selected.is_empty() {
        return Ok(prompt.to_owned());
    }
    if prompt.len().saturating_add(selected.len()).saturating_add(2) > 16384 {
        return Err("CONTEXT_CANARY_PROMPT_TOO_LARGE");
    }
    Ok(format!("{prompt}\n\n{selected}"))
}

#[cfg(test)]
mod context_handoff_gate_tests_v1 {
    use sha2::Digest;

    fn canary_packet_v170(text: &str) -> String {
        let sha = sha2::Sha256::digest(text.as_bytes())
            .iter().map(|b| format!("{b:02x}"))
            .collect::<String>();
        serde_json::json!({
            "version": "swarm-selected-context-v1",
            "taskId": "task-170",
            "planId": "plan-170",
            "excerpts": [{
                "kind": "persistent_memory",
                "id": "memory-1",
                "text": text,
                "sha256": sha
            }]
        }).to_string()
    }

    #[test]
    fn valid_selected_context_handoff_v170() {
        let evidence = "Customer memory stays on the central Swarm backend.";
        let packet = canary_packet_v170(evidence);
        let result = super::prepare_inference_prompt_v1(
            "State the retention rule.",
            Some(&packet), "task-170", true, false
        ).unwrap();

        assert!(result.starts_with(
            "State the retention rule.\n\n[SWARM_SELECTED_CONTEXT_V1]"
        ));
        assert!(result.contains(evidence));
        assert!(result.contains("\"kind\":\"persistent_memory\""));

        let wrong_task = super::prepare_inference_prompt_v1(
            "State the retention rule.",
            Some(&packet), "task-999", true, false
        );
        assert_eq!(wrong_task, Err("CONTEXT_CONTRACT_INVALID"));

        let altered = packet.replace("central Swarm", "local worker");
        let tampered = super::prepare_inference_prompt_v1(
            "State the retention rule.",
            Some(&altered), "task-170", true, false
        );
        assert_eq!(tampered, Err("CONTEXT_HASH_MISMATCH"));
    }

    #[test]
    #[ignore = "requires the local 3B llama tokenizer"]
    fn live_3b_selected_context_tokens_v170() {
        let instruction = "What is the memory retention rule?";
        let evidence = "Customer memory stays on the central Swarm backend.";
        let packet = canary_packet_v170(evidence);

        let compact = super::prepare_inference_prompt_v1(
            instruction, Some(&packet), "task-170", true, false
        ).unwrap();

        let noise = "Repeated navigation, duplicate headers, and unrelated page links. "
            .repeat(80);
        let full = format!("{instruction}\n\n{noise}\n\n{evidence}");

        let full_tokens =
            crate::core::swarm_token_context_v1::count_local_tokens(&full)
                .expect("full prompt tokenizer failed");
        let compact_tokens =
            crate::core::swarm_token_context_v1::count_local_tokens(&compact)
                .expect("compact prompt tokenizer failed");

        println!("FULL_INPUT_TOKENS={full_tokens}");
        println!("COMPACT_INPUT_TOKENS={compact_tokens}");
        println!("SYNTHETIC_TOKEN_REDUCTION_PCT={:.2}",
            100.0 * (1.0 - compact_tokens as f64 / full_tokens as f64)
        );
        assert!(compact_tokens < full_tokens);
    }

    use super::prepare_inference_prompt_v1 as prepare;

    #[test]
    fn legacy_task_unchanged() {
        assert_eq!(prepare("original", None, "t1", false, false),
                   Ok("original".to_owned()));
    }

    #[test]
    fn unsolicited_context_fails_closed() {
        assert_eq!(prepare("original", Some("{}"), "t1", false, false),
                   Err("CONTEXT_CANARY_DISABLED"));
    }

    #[test]
    fn retrieval_combination_fails_closed() {
        assert_eq!(prepare("original", Some("{}"), "t1", true, true),
                   Err("CONTEXT_RETRIEVAL_NOT_YET_SUPPORTED"));
    }

    #[test]
    fn malformed_context_is_rejected() {
        assert!(prepare("original", Some("{}"), "t1", true, false).is_err());
    }

    #[test]
    fn task_envelope_recognizes_context_field() {
        let task: crate::core::task_client::TaskEnvelope =
            serde_json::from_value(serde_json::json!({
                "taskId": "t1",
                "prompt": "original",
                "selectedContextV1": {"version": "test"}
            })).unwrap();
        assert!(task.selected_context_v1.is_some());
    }
}
