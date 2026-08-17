#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpecV1 {
    pub selected_model: &'static str,
    pub capability: &'static str,
    pub tier: u8,
    pub required_for_tier: bool,
    pub runtime: &'static str,
    pub min_ram_gb: u16,
    pub default_ctx: u32,
    pub default_max_tokens: u32,
    pub allowed_max_tokens: u32,
    pub experimental: bool,
    pub patterns: &'static [&'static str],
}

pub const MODEL_REGISTRY_VERSION: &str =
    "edgeswarm-canonical-model-registry-v1";

pub const OUTPUT_LIMIT_POLICY_VERSION: &str =
    "edgeswarm-output-limit-policy-v1";

pub const MODEL_REGISTRY_V1: &[ModelSpecV1] = &[
    ModelSpecV1 {
        selected_model: "qwen2.5:3b",
        capability: "Neural-Inference-3B",
        tier: 2,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 8,
        default_ctx: 2048,
        default_max_tokens: 256,
        allowed_max_tokens: 512,
        experimental: false,
        patterns: &[
            "*Qwen2.5-3B*Q4_K_M*.gguf",
            "*qwen2.5*3b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "qwen2.5:7b",
        capability: "Neural-Inference-7B",
        tier: 3,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 16,
        default_ctx: 4096,
        default_max_tokens: 384,
        allowed_max_tokens: 1024,
        experimental: false,
        patterns: &[
            "*Qwen2.5-7B*Q4_K_M*.gguf",
            "*qwen2.5*7b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "llama3.1:8b",
        capability: "Neural-Inference-8B",
        tier: 3,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 16,
        default_ctx: 4096,
        default_max_tokens: 384,
        allowed_max_tokens: 1024,
        experimental: false,
        patterns: &[
            "*Llama-3.1-8B*Q4_K_M*.gguf",
            "*Meta-Llama-3.1-8B*Q4_K_M*.gguf",
            "*llama*3.1*8b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "qwen2.5:14b",
        capability: "Neural-Inference-14B",
        tier: 4,
        required_for_tier: true,
        runtime: "llama.cpp",
        min_ram_gb: 32,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: 1536,
        experimental: false,
        patterns: &[
            "*Qwen2.5-14B*Q4_K_M*.gguf",
            "*qwen2.5*14b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "qwen2.5-coder:14b",
        capability: "Neural-Inference-14B",
        tier: 4,
        required_for_tier: true,
        runtime: "llama.cpp",
        min_ram_gb: 32,
        default_ctx: 4096,
        default_max_tokens: 1024,
        allowed_max_tokens: 2048,
        experimental: false,
        patterns: &[
            "*Qwen2.5-Coder-14B*Q4_K_M*.gguf",
            "*qwen2.5-coder*14b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "gemma3:27b",
        capability: "Neural-Inference-27B",
        tier: 5,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 48,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: 2048,
        experimental: false,
        patterns: &[
            "*gemma*3*27b*Q4_K_M*.gguf",
            "*gemma*27b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "mistral-small:24b",
        capability: "Neural-Inference-24B",
        tier: 5,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 48,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: 2048,
        experimental: false,
        patterns: &[
            "*Mistral-Small-24B*Q4_K_M*.gguf",
            "*mistral-small*24b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "qwen3:30b",
        capability: "Neural-Inference-30B",
        tier: 5,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 64,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: 2048,
        experimental: true,
        patterns: &[
            "*Qwen*3*30B*Q4_K_M*.gguf",
            "*qwen3*30b*q4_k_m*.gguf",
        ],
    },
];

const PRIORITY_3B: &[&str] = &["qwen2.5:3b"];
const PRIORITY_7B: &[&str] = &["qwen2.5:7b"];
const PRIORITY_8B: &[&str] = &["llama3.1:8b"];
const PRIORITY_14B: &[&str] =
    &["qwen2.5-coder:14b", "qwen2.5:14b"];
const PRIORITY_24B: &[&str] = &["mistral-small:24b"];
const PRIORITY_27B: &[&str] = &["gemma3:27b"];
const PRIORITY_30B: &[&str] = &["qwen3:30b"];

const PRIORITY_GENERIC: &[&str] = &[
    "qwen2.5-coder:14b",
    "qwen2.5:14b",
    "gemma3:27b",
    "mistral-small:24b",
    "qwen3:30b",
    "llama3.1:8b",
    "qwen2.5:7b",
    "qwen2.5:3b",
];

pub fn model_spec(
    selected_model: &str,
) -> Option<&'static ModelSpecV1> {
    MODEL_REGISTRY_V1
        .iter()
        .find(|spec| spec.selected_model == selected_model)
}

pub fn capability_priority(
    capability: &str,
) -> Option<&'static [&'static str]> {
    match capability {
        "Neural-Inference-3B" => Some(PRIORITY_3B),
        "Neural-Inference-7B" => Some(PRIORITY_7B),
        "Neural-Inference-8B" => Some(PRIORITY_8B),
        "Neural-Inference-14B" => Some(PRIORITY_14B),
        "Neural-Inference-24B" => Some(PRIORITY_24B),
        "Neural-Inference-27B" => Some(PRIORITY_27B),
        "Neural-Inference-30B" => Some(PRIORITY_30B),
        "Neural-Inference" => Some(PRIORITY_GENERIC),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_model_priority_order_is_preserved() {
        assert_eq!(
            capability_priority("Neural-Inference-14B").unwrap(),
            ["qwen2.5-coder:14b", "qwen2.5:14b"]
        );

        assert_eq!(
            capability_priority("Neural-Inference").unwrap(),
            [
                "qwen2.5-coder:14b",
                "qwen2.5:14b",
                "gemma3:27b",
                "mistral-small:24b",
                "qwen3:30b",
                "llama3.1:8b",
                "qwen2.5:7b",
                "qwen2.5:3b",
            ]
        );
    }

    #[test]
    fn every_priority_entry_exists_in_registry() {
        for capability in [
            "Neural-Inference-3B",
            "Neural-Inference-7B",
            "Neural-Inference-8B",
            "Neural-Inference-14B",
            "Neural-Inference-24B",
            "Neural-Inference-27B",
            "Neural-Inference-30B",
            "Neural-Inference",
        ] {
            for selected in capability_priority(capability).unwrap() {
                assert!(
                    model_spec(selected).is_some(),
                    "missing registry model: {selected}"
                );
            }
        }
    }

    #[test]
    fn allowed_ceiling_is_never_below_default() {
        for spec in MODEL_REGISTRY_V1 {
            assert!(
                spec.allowed_max_tokens >= spec.default_max_tokens,
                "{} allowed ceiling is below default",
                spec.selected_model
            );
        }
    }

    #[test]
    fn canonical_3b_output_budget_has_completion_headroom() {
        let spec = model_spec("qwen2.5:3b").unwrap();

        assert_eq!(spec.default_max_tokens, 256);
        assert_eq!(spec.allowed_max_tokens, 512);
    }

    #[test]
    fn canonical_3b_spec_matches_legacy_v1() {
        let spec = model_spec("qwen2.5:3b").unwrap();

        assert_eq!(spec.capability, "Neural-Inference-3B");
        assert_eq!(spec.tier, 2);
        assert_eq!(spec.runtime, "llama.cpp");
        assert_eq!(spec.min_ram_gb, 8);
        assert_eq!(spec.default_ctx, 2048);
        assert_eq!(spec.default_max_tokens, 256);
        assert!(!spec.experimental);
    }
}
