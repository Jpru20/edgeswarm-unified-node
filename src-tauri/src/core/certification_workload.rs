use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CertificationProfile {
    Normal,
    Heavy,
    Structured,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    Support,
    Summarization,
    Debugging,
    Reasoning,
    Extraction,
    InstructionFollowing,
    Sentiment,
    SupportTriage,
    EmailRewrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationSpec {
    pub required_json_keys: Vec<String>,
    pub required_terms: Vec<String>,
    pub minimum_output_chars: usize,
    pub maximum_output_chars: usize,
    #[serde(default)]
    pub expected_json_values: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificationWorkload {
    pub id: String,
    pub profile: CertificationProfile,
    pub kind: WorkloadKind,
    #[serde(default)]
    pub expected_required_model: String,
    #[serde(default)]
    pub adapter_lane: String,
    #[serde(default)]
    pub production_policy_version: String,
    pub max_output_tokens: u32,
    pub prompt: String,
    pub validation: ValidationSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificationPack {
    pub pack_id: String,
    pub pack_version: u16,
    pub model_tier: String,
    pub workloads: Vec<CertificationWorkload>,
}

impl CertificationPack {
    pub fn validate(&self) -> Result<(), String> {
        if self.workloads.len() < 6 {
            return Err("certification_pack_requires_at_least_6_workloads".into());
        }

        let mut ids = HashSet::new();

        for workload in &self.workloads {
            if !ids.insert(&workload.id) {
                return Err(format!("duplicate_workload_id:{}", workload.id));
            }

            if workload.prompt.len() < 60 {
                return Err(format!("workload_too_small:{}", workload.id));
            }

            if self.pack_version >= 2 {
                if workload.expected_required_model.is_empty() {
                    return Err(format!("missing_expected_route:{}", workload.id));
                }

                if workload.adapter_lane.is_empty() {
                    return Err(format!("missing_adapter_lane:{}", workload.id));
                }

                if workload.production_policy_version.is_empty() {
                    return Err(format!("missing_policy_version:{}", workload.id));
                }
            }

            if workload.max_output_tokens < 128 {
                return Err(format!("output_budget_too_small:{}", workload.id));
            }
        }

        for profile in [
            CertificationProfile::Normal,
            CertificationProfile::Heavy,
            CertificationProfile::Structured,
        ] {
            let count = self
                .workloads
                .iter()
                .filter(|w| w.profile == profile)
                .count();

            if count < 2 {
                return Err(format!("insufficient_profile_coverage:{profile:?}"));
            }
        }

        Ok(())
    }
}

pub fn built_in_3b_realworld_v1() -> Result<CertificationPack, String> {
    let raw = include_str!("../certification_packs/3b-realworld-v1.json");

    let pack: CertificationPack =
        serde_json::from_str(raw).map_err(|e| format!("pack_parse_failed:{e}"))?;

    pack.validate()?;
    Ok(pack)
}

pub const NEURAL_REALWORLD_PACK_ID_V1: &str = "edgeswarm-neural-realworld-v1";

pub const LEGACY_NEURAL_REALWORLD_PACK_ID_V1: &str = "edgeswarm-3b-realworld-v2";

pub fn built_in_neural_realworld_v1() -> Result<CertificationPack, String> {
    let raw = include_str!("../certification_packs/neural-realworld-v1.json");
    let pack: CertificationPack =
        serde_json::from_str(raw).map_err(|e| format!("pack_parse_failed:{e}"))?;
    pack.validate()?;
    Ok(pack)
}

pub fn bind_neural_realworld_pack_v1(
    pack: &mut CertificationPack,
    capability: &str,
) -> Result<(), String> {
    if !capability.starts_with("Neural-Inference-") {
        return Err(format!(
            "unsupported_neural_certification_capability:{capability}"
        ));
    }

    for workload in &mut pack.workloads {
        workload.expected_required_model = capability.to_string();
    }

    pack.validate()
}

pub fn built_in_3b_realworld_v2() -> Result<CertificationPack, String> {
    let raw = include_str!("../certification_packs/3b-realworld-v2.json");

    let pack: CertificationPack =
        serde_json::from_str(raw).map_err(|e| format!("pack_parse_failed:{e}"))?;

    pack.validate()?;
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_b_realworld_pack_is_valid() {
        let pack = built_in_3b_realworld_v1().expect("3B certification pack must load");

        assert_eq!(pack.pack_id, "edgeswarm-3b-realworld-v1");
        assert_eq!(pack.workloads.len(), 6);
        assert!(pack.validate().is_ok());
    }
}
