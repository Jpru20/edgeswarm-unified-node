use edgeswarm_unified_node_lib::core::{
    production_heartbeat::ProductionHeartbeatV1,
    NodeState,
};

fn main() {
    let state = NodeState::detect();

    let heartbeat =
        ProductionHeartbeatV1::from_node_state(
            &state,
            "0.1.0",
            "laptop",
            &[],
        );

    println!(
        "{}",
        serde_json::to_string_pretty(&heartbeat)
            .expect("heartbeat serialization failed")
    );

    println!();
    println!(
        "MODELS_AVAILABLE_COUNT={}",
        heartbeat.models_available.len()
    );

    println!(
        "ELIGIBLE_CAPABILITY_COUNT={}",
        heartbeat
            .eligible_model_capabilities
            .len()
    );

    println!(
        "PRIMARY_MODEL={}",
        heartbeat
            .model_id
            .as_deref()
            .unwrap_or("none")
    );

    println!(
        "PRIMARY_CAPABILITY={}",
        heartbeat
            .model_capability
            .as_deref()
            .unwrap_or("none")
    );

    println!(
        "PRODUCTION_CONCURRENCY_LIMIT={}",
        heartbeat.concurrency_limit
    );

    println!(
        "MODEL_CAPACITY_ENTRY_COUNT={}",
        heartbeat
            .metadata
            .model_capacity_v1
            .len()
    );

    assert_eq!(
        heartbeat.models_available.len(),
        8
    );

    assert_eq!(
        heartbeat.eligible_model_capabilities,
        vec!["Neural-Inference-3B"]
    );

    assert_eq!(
        heartbeat.model_id.as_deref(),
        Some("qwen2.5:3b")
    );

    assert_eq!(
        heartbeat.model_capability.as_deref(),
        Some("Neural-Inference-3B")
    );

    assert_eq!(
        heartbeat.concurrency_limit,
        1
    );

    assert_eq!(
        heartbeat.metadata.model_capacity_v1.len(),
        8
    );

    let certified = heartbeat
        .metadata
        .model_capacity_v1
        .iter()
        .filter(|model| {
            model.capacity_status
                == edgeswarm_unified_node_lib::core::
                    capacity::CapacityStatus::Certified
        })
        .count();

    assert_eq!(certified, 1);

    println!("CERTIFIED_MODEL_COUNT={certified}");
    println!("PRODUCTION_HEARTBEAT_PAYLOAD_VALID=true");
}
