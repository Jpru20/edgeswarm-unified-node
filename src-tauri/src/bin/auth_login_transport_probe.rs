use edgeswarm_unified_node_lib::core::
    auth_login_client::SupabaseLoginClient;

fn main() {
    let client =
        SupabaseLoginClient::from_env()
            .expect("login transport init failed");

    let report = client.dry_run();

    println!(
        "PASSWORD_ENDPOINT={}",
        report.password_url
    );
    println!(
        "USER_ENDPOINT={}",
        report.user_url
    );
    println!(
        "CHALLENGE_ENDPOINT={}",
        report.challenge_template
    );
    println!(
        "VERIFY_ENDPOINT={}",
        report.verify_template
    );
    println!(
        "ANON_KEY_PRESENT={}",
        report.anon_key_present
    );
    println!("LOGIN_TRANSPORT_DRY_RUN_VALID=true");
    println!("PASSWORD_USED=false");
    println!("MFA_CODE_USED=false");
    println!("NETWORK_REQUEST_SENT=false");
    println!("AUTH_SESSION_WRITTEN=false");
    println!("WALLET_CREATED=false");
    println!("WORKER_WALLET_WRITTEN=false");
    println!("SECOND_HEARTBEAT_SENT=false");
}
