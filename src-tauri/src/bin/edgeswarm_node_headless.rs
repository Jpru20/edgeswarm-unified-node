use edgeswarm_unified_node_lib::core::{
    node_service::run_node_service,
    power_guard::PowerGuard,
};
use std::{
    env,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use zeroize::Zeroizing;

const SYSTEMD_WALLET_CREDENTIAL: &str =
    "edgeswarm-wallet-password";

fn wallet_password_path() -> Result<PathBuf, String> {
    if let Some(path) =
        env::var_os("EDGESWARM_WALLET_PASSWORD_FILE")
    {
        return Ok(PathBuf::from(path));
    }

    if let Some(directory) =
        env::var_os("CREDENTIALS_DIRECTORY")
    {
        return Ok(
            PathBuf::from(directory)
                .join(SYSTEMD_WALLET_CREDENTIAL),
        );
    }

    Err("wallet_password_credential_missing".into())
}

fn read_wallet_password() -> Result<Zeroizing<String>, String> {
    let path = wallet_password_path()?;

    let raw = fs::read_to_string(&path)
        .map_err(|_| {
            "wallet_password_credential_read_failed".to_string()
        })?;

    let password = raw
        .trim_end_matches(|c| c == '\r' || c == '\n')
        .to_string();

    if password.is_empty() {
        return Err(
            "wallet_password_credential_empty".into()
        );
    }

    Ok(Zeroizing::new(password))
}

fn run() -> Result<(), String> {
    let wallet_password = read_wallet_password()?;

    let stop = Arc::new(AtomicBool::new(false));
    let signal_stop = Arc::clone(&stop);

    ctrlc::set_handler(move || {
        signal_stop.store(true, Ordering::Release);
    })
    .map_err(|_| "shutdown_signal_handler_failed".to_string())?;

    let _power_guard = PowerGuard::acquire()?;

    println!("HEADLESS_NODE_MODE=true");
    println!("HEADLESS_NODE_SIGNAL_HANDLER_READY=true");
    println!("HEADLESS_NODE_POWER_GUARD_READY=true");

    run_node_service(stop, wallet_password)
}

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "HEADLESS_NODE_ERROR={}",
            error.replace('\n', " ")
        );

        let exit_code =
            if error == "node_service_already_running" {
                73
            } else {
                1
            };

        std::process::exit(exit_code);
    }
}
