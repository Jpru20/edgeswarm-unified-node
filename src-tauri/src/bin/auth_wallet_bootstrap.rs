use edgeswarm_unified_node_lib::core::{
    auth_login_client::SupabaseLoginClient,
    auth_login_contract::{jwt_aal, verified_totp_factor},
    auth_session::AuthSession,
    hardware_identity::HardwareIdentity,
    wallet_account::DeviceWallet,
    wallet_client::WorkerWalletClient,
    wallet_identity::{select_wallet_row, WalletRowDecision},
    wallet_vault,
};
use std::{
    io::{self, Write},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

fn run() -> Result<(), String> {
    print!("EdgeSwarm email: ");
    io::stdout()
        .flush()
        .map_err(|_| "stdout_flush_failed".to_string())?;

    let mut email = String::new();
    io::stdin()
        .read_line(&mut email)
        .map_err(|_| "email_read_failed".to_string())?;
    let email = email.trim().to_lowercase();

    let password = Zeroizing::new(
        rpassword::prompt_password("Password: ").map_err(|_| "password_read_failed".to_string())?,
    );

    let login = SupabaseLoginClient::from_env()?;
    let aal1 = login.password_login(&email, &password)?;
    let user = login.get_user(&aal1.access_token)?;

    let authenticated_email = user.email.as_deref().unwrap_or("").trim().to_lowercase();

    if authenticated_email != email {
        return Err("authenticated_email_mismatch".into());
    }

    let factor_id = verified_totp_factor(&user)
        .ok_or_else(|| "verified_totp_factor_missing".to_string())?
        .id
        .clone();

    let challenge = login.challenge(&aal1.access_token, &factor_id)?;

    let code = Zeroizing::new(
        rpassword::prompt_password("6-digit authenticator code: ")
            .map_err(|_| "mfa_code_read_failed".to_string())?,
    );

    let aal2 = login.verify(&aal1.access_token, &factor_id, &challenge.id, &code)?;

    if jwt_aal(&aal2.access_token).as_deref() != Some("aal2") {
        return Err("mfa_session_not_aal2".into());
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "clock_failed".to_string())?
        .as_secs();

    let expires_at = aal2
        .expires_at
        .or_else(|| aal2.expires_in.map(|v| now.saturating_add(v)))
        .ok_or_else(|| "mfa_session_expiry_missing".to_string())?;

    let session = AuthSession::from_authenticated_session(
        &authenticated_email,
        &aal2.access_token,
        &aal2.refresh_token,
        expires_at,
    )?;
    session.save_secure()?;

    let hardware = HardwareIdentity::detect()?;
    let wallet_client = WorkerWalletClient::from_env()?;

    let rows = wallet_client.rows_for_email(&aal2.access_token, &authenticated_email)?;

    match select_wallet_row(&rows, &hardware.hardware_id)? {
        WalletRowDecision::ExactDevice { row_index } => {
            let private_key = Zeroizing::new(wallet_vault::decrypt(
                &rows[row_index].private_key,
                &password,
                &authenticated_email,
            )?);

            let wallet = DeviceWallet::from_private_key(&private_key)?;

            println!("WALLET_ACTION=reuse_exact_device");
            println!("WALLET_ADDRESS={}", wallet.wallet_address());
        }

        WalletRowDecision::ClaimLegacy { .. } => {
            return Err("legacy_wallet_claim_requires_separate_review".into());
        }

        WalletRowDecision::CreateDevice => {
            let wallet = DeviceWallet::generate()?;

            let encrypted =
                wallet_vault::encrypt(wallet.private_key(), &password, &authenticated_email)?;

            let status = wallet_client.insert_device(
                &aal2.access_token,
                &authenticated_email,
                &hardware.hardware_id,
                &encrypted,
            )?;

            let rows_after =
                wallet_client.rows_for_email(&aal2.access_token, &authenticated_email)?;

            let exact = rows_after.iter().find(|row| {
                row.hardware_id
                    .as_deref()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(&hardware.hardware_id)
            });

            let exact =
                exact.ok_or_else(|| format!("wallet_insert_not_visible_after_http_{status}"))?;

            let recovered_key = Zeroizing::new(wallet_vault::decrypt(
                &exact.private_key,
                &password,
                &authenticated_email,
            )?);

            let recovered = DeviceWallet::from_private_key(&recovered_key)?;

            if !recovered
                .wallet_address()
                .eq_ignore_ascii_case(wallet.wallet_address())
            {
                return Err("wallet_post_insert_identity_mismatch".into());
            }

            println!("WALLET_ACTION=create_new_device_wallet");
            println!("WALLET_INSERT_HTTP_STATUS={status}");
            println!("WALLET_ADDRESS={}", wallet.wallet_address());
        }
    }

    println!("SESSION_AAL2=true");
    println!("AUTH_SESSION_WRITTEN=true");
    println!("HARDWARE_ID={}", hardware.hardware_id);
    println!("PRIVATE_KEY_PRINTED=false");
    println!("PASSWORD_PRINTED=false");
    println!("PASSWORD_PERSISTED=false");
    println!("WALLET_ROW_VERIFIED=true");
    println!("SECOND_HEARTBEAT_SENT=false");
    println!("TASK_POLL_SENT=false");

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("AUTH_WALLET_BOOTSTRAP_ERROR={error}");
        std::process::exit(2);
    }
}
