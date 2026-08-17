use edgeswarm_unified_node_lib::core::{
    auth_login_client::SupabaseLoginClient,
    auth_login_contract::{jwt_aal, verified_totp_factor},
    auth_session::AuthSession,
};
use std::{
    io::{self, Write},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

fn run() -> Result<(), String> {
    print!("EdgeSwarm email: ");
    io::stdout().flush().map_err(|_| "stdout_flush_failed".to_string())?;

    let mut email = String::new();
    io::stdin()
        .read_line(&mut email)
        .map_err(|_| "email_read_failed".to_string())?;

    let email = email.trim().to_lowercase();

    if email.is_empty() {
        return Err("email_missing".into());
    }

    let password = Zeroizing::new(
        rpassword::prompt_password("Password: ")
            .map_err(|_| "password_read_failed".to_string())?
    );

    let client = SupabaseLoginClient::from_env()?;

    let password_session =
        client.password_login(&email, &password)?;

    let user =
        client.get_user(&password_session.access_token)?;

    let authenticated_email = user
        .email
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_lowercase();

    if authenticated_email.is_empty()
        || authenticated_email != email
    {
        return Err("authenticated_email_mismatch".into());
    }

    let factor_id =
        verified_totp_factor(&user)
            .ok_or_else(|| "verified_totp_factor_missing".to_string())?
            .id
            .clone();

    let challenge =
        client.challenge(
            &password_session.access_token,
            &factor_id,
        )?;

    let code = Zeroizing::new(
        rpassword::prompt_password(
            "6-digit authenticator code: "
        )
        .map_err(|_| "mfa_code_read_failed".to_string())?
    );

    let verified =
        client.verify(
            &password_session.access_token,
            &factor_id,
            &challenge.id,
            &code,
        )?;

    if jwt_aal(&verified.access_token).as_deref()
        != Some("aal2")
    {
        return Err("mfa_session_not_aal2".into());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "clock_failed".to_string())?
        .as_secs();

    let expires_at = verified
        .expires_at
        .or_else(|| {
            verified.expires_in
                .map(|seconds| now.saturating_add(seconds))
        })
        .ok_or_else(|| "mfa_session_expiry_missing".to_string())?;

    let session =
        AuthSession::from_authenticated_session(
            &authenticated_email,
            &verified.access_token,
            &verified.refresh_token,
            expires_at,
        )?;

    session.save_secure()?;

    println!("PASSWORD_LOGIN_SUCCEEDED=true");
    println!("VERIFIED_TOTP_FACTOR_FOUND=true");
    println!("MFA_CHALLENGE_VERIFIED=true");
    println!("SESSION_AAL2=true");
    println!("AUTH_SESSION_WRITTEN=true");
    println!("PASSWORD_PRINTED=false");
    println!("MFA_CODE_PRINTED=false");
    println!("TOKEN_PRINTED=false");
    println!("PASSWORD_PERSISTED=false");
    println!("WALLET_CREATED=false");
    println!("WORKER_WALLET_WRITTEN=false");
    println!("SECOND_HEARTBEAT_SENT=false");
    println!("TASK_POLL_SENT=false");

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("AUTH_INTERACTIVE_LOGIN_ERROR={error}");
        std::process::exit(2);
    }
}
