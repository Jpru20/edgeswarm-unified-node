use edgeswarm_unified_node_lib::core::{
    production_heartbeat::ProductionHeartbeatV1,
    production_heartbeat_client::
        ProductionHeartbeatClient,
    NodeState,
};

fn main() {
    let execute =
        std::env::var("EXECUTE")
            .map(|value| {
                value.eq_ignore_ascii_case("true")
            })
            .unwrap_or(false);

    let state = NodeState::detect();

    let heartbeat =
        ProductionHeartbeatV1::from_node_state(
            &state,
            "0.1.0",
            "laptop",
            &[],
        );

    let client =
        ProductionHeartbeatClient::new()
            .expect("heartbeat client init failed");

    if !execute {
        let report =
            client
                .dry_run(&heartbeat)
                .expect("heartbeat dry run failed");

        println!("EXECUTE=false");
        println!(
            "MODEL_COUNT={}",
            report.model_count
        );
        println!(
            "CERTIFIED_MODEL_COUNT={}",
            report.certified_model_count
        );
        println!(
            "CONCURRENCY_LIMIT={}",
            report.concurrency_limit
        );
        println!(
            "UNIFIED_PROTOCOL_VALID={}",
            report.unified_protocol_valid
        );
        println!("NETWORK_REQUEST_SENT=false");
        println!("UNIFIED_HEARTBEAT_SENT=false");
        return;
    }

    let result =
        client
            .send_live(&heartbeat)
            .expect("heartbeat send failed");

    println!("EXECUTE=true");
    println!(
        "HTTP_STATUS={}",
        result.status_code
    );
    println!(
        "AUTH_REFRESH_ATTEMPTED={}",
        result.auth_refresh_attempted
    );
    println!(
        "NETWORK_REQUEST_SENT={}",
        result.network_request_sent
    );
    println!(
        "UNIFIED_HEARTBEAT_SENT={}",
        result.heartbeat_sent
    );
    println!(
        "UPDATE_REQUIRED={}",
        result.update_required
    );
}
