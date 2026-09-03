use crate::core::{
    capacity::CapacityStatus, model_discovery::discover_models, task_client::TaskEnvelope,
    NodeState,
};
use std::{env, fs, path::Path};

#[derive(Debug, Clone)]
pub struct ActiveModelV1 {
    pub selected_model: String,
    pub capability: String,
    pub runtime: String,
    pub tier: u8,
}

pub fn primary_certified_model(state: &NodeState) -> Option<ActiveModelV1> {
    let mut ready = state
        .models
        .iter()
        .filter(|model| {
            model.status == "ready"
                && model.capacity_status == CapacityStatus::Certified
                && model.certified_concurrency.unwrap_or(0) > 0
                && model.capability.starts_with("Neural-Inference")
        })
        .collect::<Vec<_>>();

    ready.sort_by(|left, right| {
        right
            .tier
            .cmp(&left.tier)
            .then_with(|| left.selected_model.cmp(&right.selected_model))
    });

    ready.first().map(|model| ActiveModelV1 {
        selected_model: model.selected_model.clone(),
        capability: model.capability.clone(),
        runtime: model.runtime.clone(),
        tier: model.tier,
    })
}

pub fn task_matches_active_model(task: &TaskEnvelope, active: &ActiveModelV1) -> bool {
    let required = task.required_model.as_deref().unwrap_or("");

    let selected = task.selected_model.as_deref().unwrap_or("");

    let required_matches = required == active.capability || required == "Neural-Inference";

    let selected_matches =
        selected.is_empty() || selected == "tier:auto" || selected == active.selected_model;

    required_matches && selected_matches
}

pub fn certified_model_for_task(
    state: &NodeState,
    task: &TaskEnvelope,
    active: Option<&ActiveModelV1>,
) -> Result<Option<ActiveModelV1>, String> {
    certified_model_for_task_from_models(&state.models, task, active)
}

fn certified_model_for_task_from_models(
    models: &[crate::core::model::ModelState],
    task: &TaskEnvelope,
    active: Option<&ActiveModelV1>,
) -> Result<Option<ActiveModelV1>, String> {
    let required = task.required_model.as_deref().unwrap_or("").trim();
    let selected = task.selected_model.as_deref().unwrap_or("").trim();

    if !required.starts_with("Neural-Inference") {
        return Ok(None);
    }

    let ready = |model: &&crate::core::model::ModelState| {
        model.status == "ready"
            && model.capacity_status == CapacityStatus::Certified
            && model.certified_concurrency.unwrap_or(0) > 0
    };

    if !selected.is_empty() && selected != "tier:auto" {
        let model = models
            .iter()
            .filter(ready)
            .find(|model| model.selected_model == selected)
            .ok_or_else(|| format!("selected_model_not_certified:{selected}"))?;

        if required != "Neural-Inference" && model.capability != required {
            return Err(format!(
                "selected_model_capability_mismatch:{selected}:{required}"
            ));
        }

        return Ok(Some(ActiveModelV1 {
            selected_model: model.selected_model.clone(),
            capability: model.capability.clone(),
            runtime: model.runtime.clone(),
            tier: model.tier,
        }));
    }

    if let Some(active) = active {
        if task_matches_active_model(task, active) {
            return Ok(Some(active.clone()));
        }
    }

    let mut candidates = models
        .iter()
        .filter(ready)
        .filter(|model| {
            required == "Neural-Inference" || model.capability == required
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|a, b| {
        b.tier
            .cmp(&a.tier)
            .then_with(|| a.selected_model.cmp(&b.selected_model))
    });

    let model = candidates
        .first()
        .ok_or_else(|| format!("certified_model_not_available:{required}"))?;

    Ok(Some(ActiveModelV1 {
        selected_model: model.selected_model.clone(),
        capability: model.capability.clone(),
        runtime: model.runtime.clone(),
        tier: model.tier,
    }))
}

pub fn runtime_switch_required(
    current_selected_model: Option<&str>,
    target: &ActiveModelV1,
) -> bool {
    current_selected_model != Some(target.selected_model.as_str())
}
pub fn verify_model_path_matches_active(
    model_path: &str,
    active: &ActiveModelV1,
) -> Result<(), String> {
    let path = Path::new(model_path);

    let canonical =
        fs::canonicalize(path).map_err(|e| format!("active_model_path_canonicalize_failed:{e}"))?;

    let parent = path
        .parent()
        .ok_or_else(|| "active_model_parent_missing".to_string())?;

    let discovered = discover_models(parent)
        .into_iter()
        .find(|model| {
            fs::canonicalize(&model.path)
                .map(|candidate| candidate == canonical)
                .unwrap_or(false)
        })
        .ok_or_else(|| "active_model_not_in_registry".to_string())?;

    if discovered.selected_model != active.selected_model {
        return Err(format!(
            "active_model_path_mismatch:expected={}:actual={}",
            active.selected_model, discovered.selected_model
        ));
    }

    Ok(())
}

pub fn runtime_acceleration_label(state: &NodeState) -> String {
    if let Ok(value) = env::var("EDGESWARM_RUNTIME_ACCELERATION") {
        if !value.trim().is_empty() {
            return value.trim().to_string();
        }
    }

    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        let gpu_layers = env::var("EDGESWARM_LLAMA_GPU_LAYERS")
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(0);

        if gpu_layers > 0 {
            return "metal".into();
        }
    }

    if !state.acceleration.backend.trim().is_empty() && state.acceleration.backend != "unprobed" {
        return state.acceleration.backend.clone();
    }

    "cpu".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        capacity::CapacityStatus,
        model::ModelState,
        task_client::TaskEnvelope,
    };
    use serde_json::json;

    fn ready_model(
        selected_model: &str,
        capability: &str,
        tier: u8,
    ) -> ModelState {
        ModelState {
            selected_model: selected_model.into(),
            model_id: selected_model.into(),
            capability: capability.into(),
            tier,
            runtime: "llama.cpp".into(),
            acceleration: "cpu".into(),
            status: "ready".into(),
            capacity_status: CapacityStatus::Certified,
            certified_concurrency: Some(1),
        }
    }

    fn uncertified_model(
        selected_model: &str,
        capability: &str,
        tier: u8,
    ) -> ModelState {
        ModelState {
            selected_model: selected_model.into(),
            model_id: selected_model.into(),
            capability: capability.into(),
            tier,
            runtime: "llama.cpp".into(),
            acceleration: "cpu".into(),
            status: "installed_uncertified".into(),
            capacity_status: CapacityStatus::Uncertified,
            certified_concurrency: None,
        }
    }

    fn task(required: &str, selected: Option<&str>) -> TaskEnvelope {
        TaskEnvelope {
            task_id: json!("routing-test"),
            client_name: None,
            prompt: "test".into(),
            required_model: Some(required.into()),
            selected_model: selected.map(str::to_string),
            model_route_reason: None,
            model_routing_version: None,
            verification_seed: None,
            checkpoint_indices: Vec::new(),
            verification_method: None,
            max_output_tokens: None,
            streaming_contract: None,
        }
    }

    #[test]
    fn level2_3b_reuses_active_model() {
        let models = vec![
            ready_model("qwen2.5:3b", "Neural-Inference-3B", 2)
        ];

        let active = ActiveModelV1 {
            selected_model: "qwen2.5:3b".into(),
            capability: "Neural-Inference-3B".into(),
            runtime: "llama.cpp".into(),
            tier: 2,
        };

        let resolved = certified_model_for_task_from_models(
            &models,
            &task("Neural-Inference-3B", Some("qwen2.5:3b")),
            Some(&active),
        )
        .unwrap()
        .unwrap();

        assert_eq!(resolved.selected_model, "qwen2.5:3b");
        assert!(!runtime_switch_required(
            Some("qwen2.5:3b"),
            &resolved
        ));
    }

    #[test]
    fn exact_coder_model_is_selected() {
        let models = vec![
            ready_model("qwen2.5:14b", "Neural-Inference-14B", 4),
            ready_model("qwen2.5-coder:14b", "Neural-Inference-14B", 4),
        ];

        let resolved = certified_model_for_task_from_models(
            &models,
            &task(
                "Neural-Inference-14B",
                Some("qwen2.5-coder:14b"),
            ),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(resolved.selected_model, "qwen2.5-coder:14b");
    }

    #[test]
    fn exact_general_14b_model_is_selected() {
        let models = vec![
            ready_model("qwen2.5:14b", "Neural-Inference-14B", 4),
            ready_model("qwen2.5-coder:14b", "Neural-Inference-14B", 4),
        ];

        let resolved = certified_model_for_task_from_models(
            &models,
            &task("Neural-Inference-14B", Some("qwen2.5:14b")),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(resolved.selected_model, "qwen2.5:14b");
    }

    #[test]
    fn tier_auto_14b_selects_a_certified_pack_member() {
        let models = vec![
            ready_model("qwen2.5:14b", "Neural-Inference-14B", 4),
            ready_model("qwen2.5-coder:14b", "Neural-Inference-14B", 4),
        ];

        let resolved = certified_model_for_task_from_models(
            &models,
            &task("Neural-Inference-14B", Some("tier:auto")),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(resolved.capability, "Neural-Inference-14B");
        assert!(
            resolved.selected_model == "qwen2.5:14b"
                || resolved.selected_model == "qwen2.5-coder:14b"
        );
    }

    #[test]
    fn explicit_uncertified_model_is_rejected() {
        let models = vec![
            ready_model("qwen2.5:14b", "Neural-Inference-14B", 4),
            uncertified_model(
                "qwen2.5-coder:14b",
                "Neural-Inference-14B",
                4,
            ),
        ];

        let error = certified_model_for_task_from_models(
            &models,
            &task(
                "Neural-Inference-14B",
                Some("qwen2.5-coder:14b"),
            ),
            None,
        )
        .unwrap_err();

        assert_eq!(
            error,
            "selected_model_not_certified:qwen2.5-coder:14b"
        );
    }

    #[test]
    fn selected_model_capability_mismatch_is_rejected() {
        let models = vec![
            ready_model(
                "qwen2.5-coder:14b",
                "Neural-Inference-14B",
                4,
            ),
        ];

        let error = certified_model_for_task_from_models(
            &models,
            &task(
                "Neural-Inference-3B",
                Some("qwen2.5-coder:14b"),
            ),
            None,
        )
        .unwrap_err();

        assert!(error.starts_with(
            "selected_model_capability_mismatch:"
        ));
    }

    #[test]
    fn deterministic_task_requires_no_neural_model() {
        let models = vec![
            ready_model("qwen2.5:3b", "Neural-Inference-3B", 2)
        ];

        let resolved = certified_model_for_task_from_models(
            &models,
            &task("Exact-Extraction", None),
            None,
        )
        .unwrap();

        assert!(resolved.is_none());
    }

    #[test]
    fn switching_between_pack_members_is_detected() {
        let target = ActiveModelV1 {
            selected_model: "qwen2.5-coder:14b".into(),
            capability: "Neural-Inference-14B".into(),
            runtime: "llama.cpp".into(),
            tier: 4,
        };

        assert!(runtime_switch_required(
            Some("qwen2.5:14b"),
            &target
        ));

        assert!(!runtime_switch_required(
            Some("qwen2.5-coder:14b"),
            &target
        ));
    }
}