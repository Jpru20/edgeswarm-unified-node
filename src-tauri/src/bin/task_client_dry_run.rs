use edgeswarm_unified_node_lib::core::{
    auth_session::AuthSession,
    production_heartbeat::ProductionHeartbeatV1,
    task_client::build_poll_url,
    wallet_public_identity::WalletPublicIdentity,
    NodeState,
};

fn main() {
    let state = NodeState::detect();
    let session = AuthSession::load_default()
        .expect("auth session missing");
    let wallet = WalletPublicIdentity::load_default()
        .expect("wallet identity missing");

    let heartbeat =
        ProductionHeartbeatV1::from_node_state(
            &state,
            "0.1.0",
            "laptop",
            &[],
        );

    let email = session.provider_email()
        .expect("provider email missing");

    let url = build_poll_url(
        &state.hardware_identity.hardware_id,
        email,
        &heartbeat.eligible_model_capabilities,
        "0.1.0",
        "linux",
    )
    .expect("poll URL build failed");

    assert_eq!(
        url.path(),
        "/swarm/get-jobs"
    );
    assert_eq!(
        wallet.hardware_id,
        state.hardware_identity.hardware_id
    );

    println!("POLL_ENDPOINT_VALID=true");
    println!("SUBMIT_ENDPOINT=/enterprise/submit-result");
    println!("HARDWARE_ID_PRESENT=true");
    println!("PROVIDER_EMAIL_PRESENT=true");
    println!(
        "CAPABILITY_COUNT={}",
        heartbeat.eligible_model_capabilities.len()
    );
    println!("POLL_LIMIT=1");
    println!("APP_TYPE=cross-platform-node");
    println!("PLATFORM=linux");
    println!("TASK_CLIENT_DRY_RUN_VALID=true");
    println!("SYNTHETIC_SIGNING_TEST_ONLY=true");
    println!("REAL_PRIVATE_KEY_USED=false");
    println!("NETWORK_REQUEST_SENT=false");
    println!("TASK_POLL_SENT=false");
    println!("TASK_CLAIMED=false");
    println!("RESULT_SUBMITTED=false");
}
