use edgeswarm_unified_node_lib::core::{
    per_model_state::resolve_per_model_states,
    NodeState,
};
use std::path::Path;

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("model root required");

    let runtime = std::env::args()
        .nth(2)
        .expect("runtime path required");

    let base = NodeState::detect();

    let states = resolve_per_model_states(
        Path::new(&root),
        Path::new(&runtime),
        &base.identity.installation_id,
        &base.acceleration.backend,
    )
    .expect("per-model state resolution failed");

    println!("MODEL_COUNT={}", states.len());

    for state in &states {
        println!(
            "{} | capability={} | status={} | capacity={:?} | certified_concurrency={:?} | fingerprint={} | cache_hit={:?} | certificate={}",
            state.selected_model,
            state.capability,
            state.status,
            state.capacity_status,
            state.certified_concurrency,
            state.fingerprint_resolved,
            state.fingerprint_cache_hit,
            state.certificate_loaded
        );
    }

    let certified =
        states.iter().filter(|state| {
            state.capacity_status
                == edgeswarm_unified_node_lib::core::capacity::CapacityStatus::Certified
        }).count();

    let hashed =
        states.iter().filter(|state| {
            state.fingerprint_resolved
        }).count();

    println!();
    println!("CERTIFIED_MODEL_COUNT={certified}");
    println!("FINGERPRINT_RESOLVED_COUNT={hashed}");

    assert_eq!(states.len(), 8);
    assert_eq!(certified, 1);
    assert_eq!(hashed, 1);

    let three_b = states
        .iter()
        .find(|state| {
            state.selected_model == "qwen2.5:3b"
        })
        .expect("3B state missing");

    assert_eq!(three_b.status, "ready");
    assert_eq!(three_b.certified_concurrency, Some(1));

    println!("PER_MODEL_STATE_VALID=true");
}
