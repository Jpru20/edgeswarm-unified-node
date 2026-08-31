use edgeswarm_unified_node_lib::core::{
    auth_login_client::SupabaseLoginClient,
    auth_login_contract::{jwt_aal, verified_totp_factor},
    auth_session::AuthSession,
    hardware_identity::HardwareIdentity,
    wallet_bootstrap::bootstrap_authenticated_device_wallet_v1,
};
use std::{
    io::{self, Write},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

fn sudo_v1(args: &[&str]) -> Result<(), String> {
    let status = std::process::Command::new("sudo")
        .args(args)
        .status()
        .map_err(|_| "sudo_command_failed".to_string())?;
    if !status.success() {
        return Err(format!("sudo_command_exit_{}", status.code().unwrap_or(-1)));
    }
    Ok(())
}

fn install_service_config_v1(password: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let user = std::env::var("USER").map_err(|_| "provider_user_missing".to_string())?;
    if user == "root" || std::env::var_os("SUDO_USER").is_some() {
        return Err("run_setup_as_provider_user_without_sudo".into());
    }
    if !user
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == 95 as char || c == 45 as char)
    {
        return Err("provider_user_invalid".into());
    }

    let auth_path = AuthSession::default_path();
    let data = auth_path
        .parent()
        .ok_or_else(|| "setup_data_dir_missing".to_string())?;
    std::fs::create_dir_all(data).map_err(|_| "setup_data_dir_create_failed".to_string())?;

    let tmp = data.join(".wallet-password.setup");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|_| "wallet_credential_stage_failed".to_string())?;
    file.write_all(password.as_bytes())
        .map_err(|_| "wallet_credential_stage_write_failed".to_string())?;
    drop(file);

    let dir = format!("/etc/edgeswarm-node/{user}");
    let wallet = format!("{dir}/wallet-password");
    let envfile = format!("{dir}/node.env");
    let service = format!("edgeswarm-node-headless@{user}.service");
    let tmpstr = tmp
        .to_str()
        .ok_or_else(|| "wallet_stage_path_invalid".to_string())?;

    let result = (|| {
        sudo_v1(&["install", "-d", "-m", "0755", &dir])?;
        sudo_v1(&["install", "-m", "0600", tmpstr, &wallet])?;
        sudo_v1(&["sh","-c",&format!("printf %s\\\\n GCP_BASE_URL=https://api.edgeswarm.io > {envfile} && chmod 0644 {envfile}")])?;
        sudo_v1(&["systemctl", "daemon-reload"])?;
        sudo_v1(&["systemctl", "enable", "--now", &service])
    })();

    let _ = std::fs::remove_file(&tmp);
    result
}

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
    let hardware = HardwareIdentity::detect()?;
    bootstrap_authenticated_device_wallet_v1(
        &authenticated_email,
        &aal2.access_token,
        password.as_str(),
    )?;
    session.save_secure()?;
    install_service_config_v1(password.as_str())?;
    println!("WALLET_PUBLIC_IDENTITY_WRITTEN=true");
    println!("SESSION_AAL2=true");
    println!("AUTH_SESSION_WRITTEN=true");
    println!("HARDWARE_ID={}", hardware.hardware_id);
    println!("PRIVATE_KEY_PRINTED=false");
    println!("PASSWORD_PRINTED=false");
    println!("PASSWORD_PERSISTED_SYSTEMD_CREDENTIAL=true");
    println!("WALLET_ROW_VERIFIED=true");
    println!("SECOND_HEARTBEAT_SENT=false");
    println!("TASK_POLL_SENT=false");

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("NODE_SETUP_ERROR={error}");
        std::process::exit(2);
    }
}
