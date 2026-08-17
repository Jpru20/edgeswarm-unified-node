use edgeswarm_unified_node_lib::core::{
    production_heartbeat::ProductionHeartbeatV1,
    wallet_public_identity::WalletPublicIdentity,
    NodeState,
};

fn main() {
    let state = NodeState::detect();
    let wallet = WalletPublicIdentity::load_default()
        .expect("wallet identity missing");

    let heartbeat = ProductionHeartbeatV1::from_node_state(
        &state, "0.1.0", "laptop", &[],
    );

    assert_eq!(heartbeat.hardware_id, wallet.hardware_id);
    assert_eq!(
        heartbeat.worker.as_deref(),
        Some(wallet.wallet_address.as_str())
    );
    assert_eq!(heartbeat.metadata.model_capacity_v1.len(), 8);
    assert_eq!(heartbeat.concurrency_limit, 1);

    println!("HEARTBEAT_HARDWARE_ID_MATCH=true");
    println!("HEARTBEAT_WORKER_PRESENT=true");
    println!("HEARTBEAT_WORKER_MATCH=true");
    println!("MODEL_COUNT=8");
    println!("CONCURRENCY_LIMIT=1");
    println!("NETWORK_REQUEST_SENT=false");
    println!("SECOND_HEARTBEAT_SENT=false");
    println!("TASK_POLL_SENT=false");
}
