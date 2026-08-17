use edgeswarm_unified_node_lib::core::{
    heartbeat::UnifiedHeartbeatV1,
    NodeState,
};

fn main() {
    let state = NodeState::detect();
    let heartbeat = UnifiedHeartbeatV1::from_state(&state);

    println!(
        "{}",
        serde_json::to_string_pretty(&heartbeat)
            .expect("failed to serialize unified heartbeat")
    );
}
