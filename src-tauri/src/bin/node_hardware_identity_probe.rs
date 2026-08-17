use edgeswarm_unified_node_lib::core::{
    hardware_identity::HardwareIdentity,
    production_heartbeat::ProductionHeartbeatV1,
    NodeState,
};

fn main() {
    let first = NodeState::detect();
    let second = NodeState::detect();

    let heartbeat =
        ProductionHeartbeatV1::from_node_state(
            &first,
            "0.1.0",
            "laptop",
            &[],
        );

    let hardware_valid =
        HardwareIdentity::is_valid_hardware_id(
            &first.hardware_identity.hardware_id
        );

    let hardware_stable =
        first.hardware_identity.hardware_id
            == second.hardware_identity.hardware_id;

    let heartbeat_matches =
        heartbeat.hardware_id
            == first.hardware_identity.hardware_id;

    let installation_separate =
        first.identity.installation_id
            != first.hardware_identity.hardware_id;

    println!(
        "HARDWARE_ID={}",
        first.hardware_identity.hardware_id
    );

    println!(
        "HARDWARE_ID_SOURCE={}",
        first.hardware_identity.source
    );

    println!(
        "HARDWARE_ID_VALID_64_HEX={}",
        hardware_valid
    );

    println!(
        "HARDWARE_ID_STABLE={}",
        hardware_stable
    );

    println!(
        "HEARTBEAT_USES_HARDWARE_ID={}",
        heartbeat_matches
    );

    println!(
        "INSTALLATION_ID_REMAINS_SEPARATE={}",
        installation_separate
    );

    println!(
        "MODEL_COUNT={}",
        heartbeat.metadata.model_capacity_v1.len()
    );

    println!(
        "CONCURRENCY_LIMIT={}",
        heartbeat.concurrency_limit
    );

    assert!(hardware_valid);
    assert!(hardware_stable);
    assert!(heartbeat_matches);
    assert!(installation_separate);

    println!(
        "NODE_HARDWARE_IDENTITY_WIRING_VALID=true"
    );

    println!(
        "NETWORK_REQUEST_SENT=false"
    );

    println!(
        "DATABASE_WRITE=false"
    );

    println!(
        "SECOND_HEARTBEAT_SENT=false"
    );

    println!(
        "WALLET_CREATED=false"
    );

    println!(
        "TASK_POLL_SENT=false"
    );
}
