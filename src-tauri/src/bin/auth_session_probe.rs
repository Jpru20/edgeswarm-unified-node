use edgeswarm_unified_node_lib::core::auth_session::{
    AuthSession,
};

fn main() {
    let path = AuthSession::default_path();

    println!(
        "AUTH_FILE_EXISTS={}",
        path.is_file()
    );

    match AuthSession::load_default() {
        Ok(session) => {
            let summary = session.summary();

            println!(
                "PROVIDER_EMAIL_PRESENT={}",
                summary.provider_email_present
            );
            println!(
                "MFA_VERIFIED={}",
                summary.mfa_verified
            );
            println!(
                "ACCESS_TOKEN_PRESENT={}",
                summary.access_token_present
            );
            println!(
                "REFRESH_TOKEN_PRESENT={}",
                summary.refresh_token_present
            );
            println!(
                "SESSION_VALID_WITHOUT_REFRESH={}",
                summary.valid_without_refresh
            );
            println!(
                "AUTH_SESSION_DRY_RUN_VALID=true"
            );
        }
        Err(err) => {
            println!(
                "AUTH_SESSION_LOAD_ERROR={}",
                err
            );
            println!(
                "AUTH_SESSION_DRY_RUN_VALID=false"
            );
        }
    }

    println!("TOKEN_VALUE_PRINTED=false");
    println!("NETWORK_REQUEST_SENT=false");
    println!("AUTH_SESSION_WRITTEN=false");
    println!("UNIFIED_HEARTBEAT_SENT=false");
}
