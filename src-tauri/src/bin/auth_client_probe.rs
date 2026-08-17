use edgeswarm_unified_node_lib::core::auth_client::{
    SupabaseAuthClient,
};

fn main() {
    let result =
        SupabaseAuthClient::from_env()
            .and_then(|client| {
                client.ensure_valid_session(false)
            });

    match result {
        Ok(state) => {
            println!(
                "AUTH_SESSION_VALID=true"
            );
            println!(
                "PROVIDER_EMAIL_PRESENT={}",
                state
                    .session
                    .provider_email()
                    .is_some()
            );
            println!(
                "AUTH_SESSION_REFRESHED={}",
                state.refreshed
            );
            println!(
                "NETWORK_REQUEST_SENT={}",
                state.network_request_sent
            );
            println!(
                "AUTH_CLIENT_DRY_RUN_VALID=true"
            );
        }

        Err(err) => {
            println!(
                "AUTH_CLIENT_ERROR={}",
                err
            );
            println!(
                "AUTH_CLIENT_DRY_RUN_VALID=false"
            );
            println!(
                "NETWORK_REQUEST_SENT=false"
            );
        }
    }

    println!("TOKEN_VALUE_PRINTED=false");
    println!("AUTH_SESSION_WRITTEN=false");
    println!("UNIFIED_HEARTBEAT_SENT=false");
}
