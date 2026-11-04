use crate::core::signed_model_registry::active_or_cached_model_registry_v171;

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

pub const MODEL_REGISTRY_VERSION: &str = "edgeswarm-canonical-model-registry-v2";

pub const OUTPUT_LIMIT_POLICY_VERSION: &str = "edgeswarm-output-limit-policy-v2";

pub const MODEL_REGISTRY_V1: &[ModelSpecV1] = &[
    ModelSpecV1 {
        selected_model: "lfm3:1.7b",
        capability: "Neural-Inference-3B",
        tier: 2,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 8,
        default_ctx: 2048,
        default_max_tokens: 256,
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*LFM3-1.7B*Q4_K_M*.gguf",
            "*lfm3*1.7b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "qwen2.5:3b",
        capability: "Neural-Inference-3B",
        tier: 2,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 8,
        default_ctx: 2048,
        default_max_tokens: 256,
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &["*Qwen2.5-3B*Q4_K_M*.gguf", "*qwen2.5*3b*q4_k_m*.gguf"],
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
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &["*Qwen2.5-7B*Q4_K_M*.gguf", "*qwen2.5*7b*q4_k_m*.gguf"],
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
        allowed_max_tokens: u32::MAX,
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
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &["*Qwen2.5-14B*Q4_K_M*.gguf", "*qwen2.5*14b*q4_k_m*.gguf"],
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
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*Qwen2.5-Coder-14B*Q4_K_M*.gguf",
            "*qwen2.5-coder*14b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "gpt-oss:20b",
        capability: "Neural-Inference-14B",
        tier: 4,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 32,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*gpt-oss-20b*MXFP4*.gguf",
            "*gpt*oss*20b*mxfp4*.gguf",
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
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &["*gemma*3*27b*Q4_K_M*.gguf", "*gemma*27b*q4_k_m*.gguf"],
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
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*Mistral-Small-24B*Q4_K_M*.gguf",
            "*mistral-small*24b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "qwen3.6:35b-a3b",
        capability: "Neural-Inference-30B",
        tier: 5,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 64,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*Qwen3.6-35B-A3B*Q4_K_M*.gguf",
            "*qwen3.6*35b*a3b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "nemotron3.5-bolt:74b-a9b",
        capability: "Neural-Inference-30B",
        tier: 5,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 64,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*NVIDIA-Nemotron-3.5-Bolt-74B-A9B-24GB*.gguf",
            "*nemotron*3.5*bolt*74b*a9b*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "muse-glimmer:30b",
        capability: "Neural-Inference-30B",
        tier: 5,
        required_for_tier: false,
        runtime: "llama.cpp",
        min_ram_gb: 48,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*Muse-Glimmer-30B*Q4_K_M*.gguf",
            "*muse*glimmer*30b*q4_k_m*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "nemotron3.5-super-vl:120b-a12b",
        capability: "Neural-Inference-70B-Plus",
        tier: 6,
        required_for_tier: true,
        runtime: "llama.cpp",
        min_ram_gb: 128,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*NVIDIA-Nemotron-3.5-Super-VL-120B-A12B-NVFP4*.gguf",
            "*nemotron*3.5*super*120b*a12b*nvfp4*.gguf",
        ],
    },
    ModelSpecV1 {
        selected_model: "llama3.1:70b",
        capability: "Neural-Inference-70B-Plus",
        tier: 6,
        required_for_tier: true,
        runtime: "llama.cpp",
        min_ram_gb: 64,
        default_ctx: 4096,
        default_max_tokens: 512,
        allowed_max_tokens: u32::MAX,
        experimental: false,
        patterns: &[
            "*Llama-3.1-70B*Q4_K_M*.gguf",
            "*Meta-Llama-3.1-70B*Q4_K_M*.gguf",
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
        allowed_max_tokens: u32::MAX,
        experimental: true,
        patterns: &["*Qwen*3*30B*Q4_K_M*.gguf", "*qwen3*30b*q4_k_m*.gguf"],
    },
];

const PRIORITY_3B: &[&str] = &[
    "lfm3:1.7b",
    "qwen2.5:3b",
];
const PRIORITY_7B: &[&str] = &["qwen2.5:7b"];
const PRIORITY_8B: &[&str] = &["llama3.1:8b"];
const PRIORITY_14B: &[&str] = &[
    "qwen2.5-coder:14b",
    "qwen2.5:14b",
    "gpt-oss:20b",
];
const PRIORITY_24B: &[&str] = &["mistral-small:24b"];
const PRIORITY_27B: &[&str] = &["gemma3:27b"];
const PRIORITY_30B: &[&str] = &[
    "qwen3.6:35b-a3b",
    "qwen3:30b",
    "nemotron3.5-bolt:74b-a9b",
    "muse-glimmer:30b",
];
const PRIORITY_70B: &[&str] = &[
    "nemotron3.5-super-vl:120b-a12b",
    "llama3.1:70b",
];

const PRIORITY_GENERIC: &[&str] = &[
    "nemotron3.5-super-vl:120b-a12b",
    "llama3.1:70b",
    "qwen3.6:35b-a3b",
    "qwen3:30b",
    "nemotron3.5-bolt:74b-a9b",
    "muse-glimmer:30b",
    "gemma3:27b",
    "mistral-small:24b",
    "qwen2.5-coder:14b",
    "qwen2.5:14b",
    "gpt-oss:20b",
    "llama3.1:8b",
    "qwen2.5:7b",
    "lfm3:1.7b",
    "qwen2.5:3b",
];

const PRIMARY_MODEL_PRIORITY_V171:
    &[&str] =
    PRIORITY_GENERIC;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
)]
pub struct EffectiveModelSpecV171 {
    pub selected_model: String,
    pub artifact_id: Option<String>,
    pub capability: String,
    pub vision_capability: Option<String>,
    pub tier: u8,
    pub runtime: String,
    pub min_ram_gb: u16,
    pub default_ctx: u32,
    pub default_max_tokens: u32,
    pub allowed_max_tokens: u32,
    pub experimental: bool,
    pub priority: u16,
    pub quantization: String,
    pub patterns: Vec<String>,
}

fn builtin_primary_model_priority_v171(
    selected_model: &str,
) -> usize {
    PRIMARY_MODEL_PRIORITY_V171
        .iter()
        .position(|candidate| {
            *candidate == selected_model
        })
        .unwrap_or(usize::MAX)
}

fn builtin_quantization_v171(
    selected_model: &str,
) -> &'static str {
    match selected_model {
        "gpt-oss:20b" =>
            "MXFP4",

        "nemotron3.5-super-vl:120b-a12b" =>
            "NVFP4",

        _ =>
            "Q4_K_M",
    }
}

pub fn effective_model_specs_v171()
    -> Vec<EffectiveModelSpecV171>
{
    if let Some(registry) =
        active_or_cached_model_registry_v171()
    {
        return registry
            .payload
            .models
            .into_iter()
            .map(|model| {
                EffectiveModelSpecV171 {
                    selected_model:
                        model.selected_model,
                    artifact_id:
                        Some(
                            model.artifact_id
                        ),
                    capability:
                        model.capability,
                    vision_capability:
                        model.vision_capability,
                    tier:
                        model.tier,
                    runtime:
                        model.runtime,
                    min_ram_gb:
                        model.min_ram_gb,
                    default_ctx:
                        model.default_ctx,
                    default_max_tokens:
                        model.default_max_tokens,
                    allowed_max_tokens:
                        u32::MAX,
                    experimental:
                        model.experimental,
                    priority:
                        model.priority,
                    quantization:
                        model.quantization,
                    patterns:
                        model.patterns,
                }
            })
            .collect();
    }

    MODEL_REGISTRY_V1
        .iter()
        .map(|spec| {
            EffectiveModelSpecV171 {
                selected_model:
                    spec.selected_model
                        .to_string(),
                artifact_id:
                    None,
                capability:
                    spec.capability
                        .to_string(),
                vision_capability:
                    None,
                tier:
                    spec.tier,
                runtime:
                    spec.runtime
                        .to_string(),
                min_ram_gb:
                    spec.min_ram_gb,
                default_ctx:
                    spec.default_ctx,
                default_max_tokens:
                    spec.default_max_tokens,
                allowed_max_tokens:
                    u32::MAX,
                experimental:
                    spec.experimental,
                priority:
                    builtin_primary_model_priority_v171(
                        spec.selected_model,
                    )
                    .min(
                        u16::MAX as usize
                    )
                    as u16,
                quantization:
                    builtin_quantization_v171(
                        spec.selected_model,
                    )
                    .to_string(),
                patterns:
                    spec.patterns
                        .iter()
                        .map(|pattern| {
                            (*pattern)
                                .to_string()
                        })
                        .collect(),
            }
        })
        .collect()
}

pub fn effective_model_spec_v171(
    selected_model: &str,
) -> Option<EffectiveModelSpecV171>
{
    effective_model_specs_v171()
        .into_iter()
        .find(|spec| {
            spec.selected_model
                == selected_model
        })
}

pub fn effective_capability_priority_v171(
    capability: &str,
) -> Vec<String> {
    let generic =
        capability == "Neural-Inference";

    let mut candidates =
        effective_model_specs_v171()
            .into_iter()
            .filter(|spec| {
                if generic {
                    spec.capability
                        .starts_with(
                            "Neural-Inference"
                        )
                } else {
                    spec.capability
                        == capability
                }
            })
            .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        if generic {
            right
                .tier
                .cmp(&left.tier)
                .then_with(|| {
                    left.priority
                        .cmp(
                            &right.priority
                        )
                })
                .then_with(|| {
                    left.selected_model
                        .cmp(
                            &right.selected_model
                        )
                })
        } else {
            left
                .priority
                .cmp(&right.priority)
                .then_with(|| {
                    left.selected_model
                        .cmp(
                            &right.selected_model
                        )
                })
        }
    });

    candidates
        .into_iter()
        .map(|spec| {
            spec.selected_model
        })
        .collect()
}

pub fn model_rank_for_capability_v171(
    capability: &str,
    selected_model: &str,
) -> usize {
    effective_capability_priority_v171(
        capability,
    )
    .iter()
    .position(|candidate| {
        candidate == selected_model
    })
    .unwrap_or(usize::MAX)
}

pub fn primary_model_priority_v171(
    selected_model: &str,
) -> usize {
    effective_model_spec_v171(
        selected_model,
    )
    .map(|spec| {
        spec.priority as usize
    })
    .unwrap_or_else(|| {
        builtin_primary_model_priority_v171(
            selected_model,
        )
    })
}

pub fn model_spec(selected_model: &str) -> Option<&'static ModelSpecV1> {
    MODEL_REGISTRY_V1
        .iter()
        .find(|spec| spec.selected_model == selected_model)
}

pub fn capability_priority(capability: &str) -> Option<&'static [&'static str]> {
    match capability {
        "Neural-Inference-3B" => Some(PRIORITY_3B),
        "Neural-Inference-7B" => Some(PRIORITY_7B),
        "Neural-Inference-8B" => Some(PRIORITY_8B),
        "Neural-Inference-14B" => Some(PRIORITY_14B),
        "Neural-Inference-24B" => Some(PRIORITY_24B),
        "Neural-Inference-27B" => Some(PRIORITY_27B),
        "Neural-Inference-30B" => Some(PRIORITY_30B),
        "Neural-Inference-70B-Plus" => Some(PRIORITY_70B),
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
            [
                "qwen2.5-coder:14b",
                "qwen2.5:14b",
                "gpt-oss:20b",
            ]
        );

        assert_eq!(
            capability_priority("Neural-Inference").unwrap(),
            [
                "nemotron3.5-super-vl:120b-a12b",
                "llama3.1:70b",
                "qwen3.6:35b-a3b",
                "qwen3:30b",
                "nemotron3.5-bolt:74b-a9b",
                "muse-glimmer:30b",
                "gemma3:27b",
                "mistral-small:24b",
                "qwen2.5-coder:14b",
                "qwen2.5:14b",
                "gpt-oss:20b",
                "llama3.1:8b",
                "qwen2.5:7b",
                "lfm3:1.7b",
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
        assert_eq!(spec.allowed_max_tokens, u32::MAX);
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
