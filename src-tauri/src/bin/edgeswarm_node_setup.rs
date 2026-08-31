use edgeswarm_unified_node_lib::core::{
    auth_login_client::SupabaseLoginClient,
    auth_login_contract::{jwt_aal, verified_totp_factor},
    auth_session::AuthSession,
    hardware_identity::HardwareIdentity,
    wallet_bootstrap::bootstrap_authenticated_device_wallet_v1,
};
use std::{
    io::{self, BufRead, BufReader, Write},
    process::{Command, Stdio},
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

fn install_service_config_v1(password: &str) -> Result<u64, String> {
    use std::io::Write;

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
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
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

        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "service_start_clock_failed".to_string())?
            .as_secs();

        sudo_v1(&["systemctl", "enable", "--now", &service])?;
        Ok(started_at)
    })();

    let _ = std::fs::remove_file(&tmp);
    result
}

fn render_console_event_v1(line: &str) {
    if let Some(v) = line.strip_prefix("MODEL_RECOMMENDATION=") {
        println!("✓ Model selected: {v}");
    } else if line == "MODEL_PROVISIONING_STARTED=true" {
        println!("↻ Downloading required AI model...");
    } else if let Some(v) = line.strip_prefix("MODEL_DOWNLOAD_PROGRESS=") {
        let p: Vec<&str> = v.split("|").collect();
        if p.len() == 6 {
            let pct = p[3].parse::<f64>().unwrap_or(0.0);
            let done = p[1].parse::<f64>().unwrap_or(0.0) / 1_073_741_824.0;
            let total = p[2].parse::<f64>().unwrap_or(0.0) / 1_073_741_824.0;
            let speed = p[4].parse::<f64>().unwrap_or(0.0) / 1_048_576.0;
            let eta = p[5].parse::<u64>().unwrap_or(0);
            let filled = ((pct / 100.0) * 24.0).round() as usize;
            let bar = format!(
                "{}{}",
                "█".repeat(filled.min(24)),
                "░".repeat(24usize.saturating_sub(filled))
            );
            println!(
                "  [{bar}] {pct:5.1}% · {done:.2}/{total:.2} GB · {speed:.1} MB/s · ETA {eta}s"
            );
        }
    } else if line == "MODEL_PROVISIONING_COMPLETE=true" {
        println!("✓ Model downloaded and verified");
    } else if line == "MODEL_CERTIFICATION_STARTED=true" {
        println!("↻ Certifying device capacity...");
    } else if let Some(v) = line.strip_prefix("CERTIFICATION_CONCURRENCY_STARTED=") {
        if v == "1" {
            println!("  Baseline · 1 worker");
        } else {
            println!("  Parallel capacity · {v} workers");
        }
    } else if let Some(v) = line.strip_prefix("CERTIFICATION_WORKLOAD_PROGRESS=") {
        let p: Vec<&str> = v.split("|").collect();
        if p.len() == 3 {
            println!("    ✓ Workload {} / {}", p[1], p[2]);
        }
    } else if line == "MODEL_CERTIFICATION_COMPLETE=true" {
        println!("✓ Device certification complete");
    } else if let Some(v) = line.strip_prefix("ACTIVE_EXECUTION_MODEL=") {
        println!("✓ Active model: {v}");
    } else if line == "LOCAL_RUNTIME_READY=true" {
        println!("✓ Local inference runtime ready");
    } else if line == "READINESS_HEARTBEAT_HTTP_STATUS=200" {
        println!("✓ Connected to EdgeSwarm");
        println!("● ONLINE — EdgeSwarm Node is ready");
        println!("Waiting for eligible work...");
    } else if line == "NODE_WAITING_FOR_ASSIGNMENT_APPROVAL=true" {
        println!("● ONLINE — Waiting for assignment approval");
    } else if line == "POLL_ASSIGNMENT_ELIGIBILITY_RESTORED=true" {
        println!("✓ Assignment eligibility enabled");
        println!("Waiting for eligible work...");
    } else if line == "TASK_CLAIMED=true" {
        println!("→ Task received");
    } else if let Some(v) = line.strip_prefix("HEADLESS_NODE_ERROR=") {
        println!("✗ Node error: {v}");
    }
}

fn show_current_node_state_v1(service: &str) {
    let active = Command::new("systemctl")
        .args(["is-active", service])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false);

    if !active {
        println!("● OFFLINE — EdgeSwarm Node service is not running");
        return;
    }

    let pid = Command::new("systemctl")
        .args(["show", service, "-p", "MainPID", "--value"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let current = if pid.is_empty() || pid == "0" {
        String::new()
    } else {
        Command::new("journalctl")
            .arg(format!("_PID={pid}"))
            .args(["--no-pager", "-o", "cat"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default()
    };

    let mut ready = false;
    let mut blocked = false;

    for line in current.lines() {
        let line = line.trim();
        if line == "READINESS_HEARTBEAT_HTTP_STATUS=200" {
            ready = true;
        } else if line == "POLL_BLOCKED=true" {
            blocked = true;
        } else if line == "POLL_BLOCKED=false" {
            blocked = false;
        }
    }

    if ready && blocked {
        println!("● ONLINE — Waiting for assignment approval");
    } else if ready {
        println!("● ONLINE — EdgeSwarm Node is ready");
    } else {
        println!("● STARTING — EdgeSwarm Node is initializing");
    }
}

fn live_console_v1(started_at: Option<u64>) -> Result<(), String> {
    let user = std::env::var("USER").map_err(|_| "provider_user_missing".to_string())?;
    let service = format!("edgeswarm-node-headless@{user}.service");

    println!();
    println!("EdgeSwarm Node Console");
    println!("================================================");
    println!("The node runs independently under systemd.");
    println!("Press Ctrl+C anytime to close this console.");
    println!("Closing the console does NOT stop the node.");
    println!("================================================");
    println!();

    show_current_node_state_v1(&service);
    println!();

    let mut command = Command::new("journalctl");
    command.args(["-u", &service, "-f", "--no-pager", "-o", "cat"]);

    if let Some(epoch) = started_at {
        command.args(["--since", &format!("@{epoch}")]);
    } else {
        command.args(["-n", "0"]);
    }

    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "node_console_journal_start_failed".to_string())?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "node_console_journal_stdout_missing".to_string())?;
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        let line = line.map_err(|_| "node_console_journal_read_failed".to_string())?;
        render_console_event_v1(line.trim());
    }

    println!();
    println!("Console detached. EdgeSwarm Node continues running.");
    Ok(())
}

fn run() -> Result<(), String> {
    let status_mode = std::env::args().any(|arg| arg == "--status")
        || std::env::args()
            .next()
            .map(|arg| arg.ends_with("edgeswarm-node-status"))
            .unwrap_or(false);

    if status_mode {
        return live_console_v1(None);
    }

    println!("EdgeSwarm Node Setup");
    println!("================================================");

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
    let service_started_at = install_service_config_v1(password.as_str())?;
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
    println!();
    println!("✓ Account verified");
    println!("✓ MFA verified");
    println!("✓ Device wallet ready");
    println!("✓ Node service configured and started");

    live_console_v1(Some(service_started_at))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("NODE_SETUP_ERROR={error}");
        std::process::exit(2);
    }
}
