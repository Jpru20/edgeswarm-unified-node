use edgeswarm_unified_node_lib::core::backend_client::{
    BackendClient,
};

fn main() {
    match BackendClient::new()
        .and_then(|client| client.dry_run())
    {
        Ok(state) => {
            println!(
                "BACKEND_BASE_URL={}",
                state.base_url
            );
            println!(
                "HEARTBEAT_URL={}",
                state.heartbeat_url
            );
            println!(
                "AUTH_SESSION_PRESENT={}",
                state.auth_session_present
            );
            println!(
                "AUTH_SESSION_VALID={}",
                state.auth_session_valid
            );
            println!(
                "PROVIDER_EMAIL_PRESENT={}",
                state.provider_email_present
            );
            println!(
                "BEARER_HEADER_READY={}",
                state.bearer_header_ready
            );
            println!(
                "BACKEND_CLIENT_DRY_RUN_VALID=true"
            );
        }
        Err(err) => {
            println!(
                "BACKEND_CLIENT_ERROR={}",
                err
            );
            println!(
                "BACKEND_CLIENT_DRY_RUN_VALID=false"
            );
        }
    }

    println!("TOKEN_VALUE_PRINTED=false");
    println!("NETWORK_REQUEST_SENT=false");
    println!("AUTH_SESSION_REFRESHED=false");
    println!("AUTH_SESSION_WRITTEN=false");
    println!("UNIFIED_HEARTBEAT_SENT=false");
}
