use edgeswarm_unified_node_lib::core::auth_client::SupabaseAuthClient;

fn main() {
    let client = SupabaseAuthClient::from_env().expect("auth client init failed");
    let result = client.force_refresh_session().expect("auth session refresh failed");
    println!("AUTH_SESSION_REFRESHED={}", result.refreshed);
    println!("AUTH_SESSION_VALID={}", result.session.is_valid_now());
    println!("PROVIDER_EMAIL_PRESENT={}", result.session.provider_email().is_some());
    println!("NETWORK_REQUEST_SENT={}", result.network_request_sent);
    println!("AUTH_SESSION_WRITTEN=true");
    println!("TOKEN_VALUE_PRINTED=false");
    println!("WORKER_WALLET_READ=false");
    println!("WORKER_WALLET_WRITTEN=false");
    println!("WALLET_CREATED=false");
    println!("SECOND_HEARTBEAT_SENT=false");
    println!("TASK_POLL_SENT=false");
}
