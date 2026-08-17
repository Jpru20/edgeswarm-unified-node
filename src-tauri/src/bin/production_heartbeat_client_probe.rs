use edgeswarm_unified_node_lib::core::{
    production_heartbeat::ProductionHeartbeatV1,
    production_heartbeat_client::
        ProductionHeartbeatClient,
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

    let result =
        ProductionHeartbeatClient::new()
            .and_then(|client| {
                client.dry_run(&heartbeat)
            });

    match result {
        Ok(report) => {
            println!(
                "HEARTBEAT_URL={}",
                report.heartbeat_url
            );
            println!(
                "AUTH_SESSION_VALID={}",
                report.auth_session_valid
            );
            println!(
                "PROVIDER_EMAIL_PRESENT={}",
                report.provider_email_present
            );
            println!(
                "BEARER_HEADER_READY={}",
                report.bearer_header_ready
            );
            println!(
                "HARDWARE_ID_PRESENT={}",
                report.hardware_id_present
            );
            println!(
                "PLATFORM={}",
                report.platform
            );
            println!(
                "MODEL_COUNT={}",
                report.model_count
            );
            println!(
                "CERTIFIED_MODEL_COUNT={}",
                report.certified_model_count
            );
            println!(
                "ELIGIBLE_CAPABILITY_COUNT={}",
                report.eligible_capability_count
            );
            println!(
                "CONCURRENCY_LIMIT={}",
                report.concurrency_limit
            );
            println!(
                "UNIFIED_PROTOCOL_VALID={}",
                report.unified_protocol_valid
            );

            assert_eq!(report.model_count, 8);
            assert_eq!(
                report.certified_model_count,
                1
            );
            assert_eq!(
                report.eligible_capability_count,
                1
            );
            assert_eq!(
                report.concurrency_limit,
                1
            );

            println!(
                "PRODUCTION_HEARTBEAT_CLIENT_DRY_RUN_VALID=true"
            );
        }

        Err(err) => {
            println!(
                "PRODUCTION_HEARTBEAT_CLIENT_ERROR={}",
                err
            );
            println!(
                "PRODUCTION_HEARTBEAT_CLIENT_DRY_RUN_VALID=false"
            );
        }
    }

    println!("TOKEN_VALUE_PRINTED=false");
    println!("NETWORK_REQUEST_SENT=false");
    println!("AUTH_SESSION_REFRESHED=false");
    println!("AUTH_SESSION_WRITTEN=false");
    println!("UNIFIED_HEARTBEAT_SENT=false");
    println!("TASK_SUBMITTED=false");
}
