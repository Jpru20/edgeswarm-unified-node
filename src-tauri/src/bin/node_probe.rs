use edgeswarm_unified_node_lib::core::NodeState;

fn main() {
    let state = NodeState::detect();

    println!(
        "{}",
        serde_json::to_string_pretty(&state)
            .expect("failed to serialize EdgeSwarm node state")
    );
}
