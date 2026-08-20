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
