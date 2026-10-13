#[derive(Debug, Clone)]
pub struct GenerationSettings {
    pub mode: String,
    pub max_tokens: u32,
    pub temperature: f64,
    pub top_p: f64,
    pub stop: Vec<String>,
}

const QWEN_EXACT_MAX_TOKENS: u32 = 120;

fn is_code_generation_prompt_v2(prompt: &str) -> bool {
    let text = prompt.to_lowercase();

    let wants_code_only = [
        "return code only",
        "code only",
        "no markdown fences",
        "no language label",
    ]
    .iter()
    .any(|token| text.contains(token));

    let looks_like_code_task = [
        "react",
        "component",
        "javascript",
        "typescript",
        "jsx",
        "tsx",
        "code",
        "function",
        "fetch",
        "api",
        "export default",
    ]
    .iter()
    .any(|token| text.contains(token));

    wants_code_only && looks_like_code_task
}

fn is_json_response_prompt_v2(prompt: &str) -> bool {
    let text = prompt.to_lowercase();

    [
        "return valid json only",
        "return json only",
        "use keys:",
        "\"summary\"",
        "\"risks\"",
        "\"recommended_fixes\"",
        "\"next_actions\"",
    ]
    .iter()
    .any(|token| text.contains(token))
}

pub fn production_generation_settings_v1(
    prompt: &str,
    task_mode: Option<&str>,
    max_tokens: Option<u32>,
) -> GenerationSettings {
    let mode = task_mode.unwrap_or("").trim().to_lowercase();

    // Respect the task's requested budget for customer-facing generation.
    // Exact extraction retains a small structured-output safety boundary.
    let (generation_mode, budget, temperature, top_p) = if mode == "exact_extraction" {
        ("exact_extraction", max_tokens.unwrap_or(QWEN_EXACT_MAX_TOKENS).max(1).min(QWEN_EXACT_MAX_TOKENS), 0.0, 0.1)
    } else if is_code_generation_prompt_v2(prompt) {
        ("code", max_tokens.unwrap_or(4096).max(1), 0.10, 0.85)
    } else if is_json_response_prompt_v2(prompt) {
        ("json", max_tokens.unwrap_or(4096).max(1), 0.05, 0.75)
    } else {
        ("general", max_tokens.unwrap_or(4096).max(1), 0.15, 0.8)
    };
    let budget = budget;

    GenerationSettings {
        mode: generation_mode.into(),
        max_tokens: budget.max(1),
        temperature,
        top_p,
        stop: vec!["<|im_end|>".into(), "<|endoftext|>".into()],
    }
}

// ADAPTIVE_GENERATION_BUDGET_V1
// Legacy tasks retain the historical mode-specific ceilings.
// adaptive-execution-v1 makes the backend-provided output budget
// authoritative while preserving task-specific generation settings.
pub fn production_generation_settings_adaptive_v1(
    prompt: &str,
    task_mode: Option<&str>,
    max_tokens: Option<u32>,
) -> GenerationSettings {
    let mode =
        task_mode
            .unwrap_or("")
            .trim()
            .to_lowercase();

    let (
        generation_mode,
        temperature,
        top_p,
    ) = if mode == "exact_extraction" {
        (
            "exact_extraction",
            0.0,
            0.1,
        )
    } else if is_code_generation_prompt_v2(prompt) {
        (
            "code",
            0.10,
            0.85,
        )
    } else if is_json_response_prompt_v2(prompt) {
        (
            "json",
            0.05,
            0.75,
        )
    } else {
        (
            "general",
            0.15,
            0.8,
        )
    };

    let mut budget =
        max_tokens
            .unwrap_or(4096)
            .max(1);

    // Exact extraction is intrinsically small. Preserve the
    // specialized output boundary rather than encouraging prose.
    if generation_mode == "exact_extraction" {
        budget =
            budget.min(
                QWEN_EXACT_MAX_TOKENS
            );
    }

    GenerationSettings {
        mode:
            generation_mode.into(),
        max_tokens:
            budget.max(1),
        temperature,
        top_p,
        stop:
            vec![
                "<|im_end|>".into(),
                "<|endoftext|>".into(),
            ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        certification_workload::built_in_3b_realworld_v2,
        production_prompt::compile_certification_prompt,
    };

    #[test]
    fn legacy_general_no_longer_caps_at_384() {
        let settings = production_generation_settings_v1(
            "Write a detailed 1500 word explanation.", None, Some(4096),
        );
        assert_eq!(settings.max_tokens, 4096);
    }

    #[test]
    fn legacy_code_and_json_honor_request_budget() {
        assert_eq!(production_generation_settings_v1(
            "Return code only. Build a React component.", None, Some(4096),
        ).max_tokens, 4096);
        assert_eq!(production_generation_settings_v1(
            "Return valid JSON only using keys: summary.", None, Some(4096),
        ).max_tokens, 4096);
    }

    #[test]
    fn adaptive_general_honors_backend_budget() {
        let settings =
            production_generation_settings_adaptive_v1(
                "Plan a detailed three-day Lisbon itinerary.",
                None,
                Some(4096),
            );

        assert_eq!(settings.mode, "general");
        assert_eq!(settings.max_tokens, 4096);
    }

    #[test]
    fn adaptive_code_is_not_limited_to_legacy_1024() {
        let settings =
            production_generation_settings_adaptive_v1(
                "Return code only. Build a React component.",
                None,
                Some(4096),
            );

        assert_eq!(settings.mode, "code");
        assert_eq!(settings.max_tokens, 4096);
    }

    #[test]
    fn adaptive_json_is_not_limited_to_legacy_640() {
        let settings =
            production_generation_settings_adaptive_v1(
                "Return valid JSON only using keys: \"summary\", \"risks\".",
                None,
                Some(4096),
            );

        assert_eq!(settings.mode, "json");
        assert_eq!(settings.max_tokens, 4096);
    }

    #[test]
    fn adaptive_explicit_one_sentence_is_prompt_not_hidden_token_cap() {
        let settings =
            production_generation_settings_adaptive_v1(
                "Answer in one concise sentence.",
                None,
                Some(4096),
            );

        assert_eq!(settings.max_tokens, 4096);
    }

    #[test]
    fn three_b_v2_uses_production_json_generation_policy() {
        let pack = built_in_3b_realworld_v2().unwrap();
        let workload = &pack.workloads[0];

        let compiled = compile_certification_prompt(workload).unwrap();

        let settings = production_generation_settings_v1(
            &compiled.user_text,
            None,
            Some(workload.max_output_tokens),
        );

        assert_eq!(settings.mode, "json");
        assert_eq!(settings.max_tokens, workload.max_output_tokens);
        assert!((settings.temperature - 0.05).abs() < 0.0001);
        assert!((settings.top_p - 0.75).abs() < 0.0001);
    }
}
