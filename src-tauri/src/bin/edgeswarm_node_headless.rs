#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use edgeswarm_unified_node_lib::core::{
    node_service::run_node_service,
    power_guard::PowerGuard,
};

#[cfg(target_os = "windows")]
use edgeswarm_unified_node_lib::core::{
    desired_state::{
        effective_desired_node_state_v1,
        DesiredNodeStateV1,
    },
    node_status_bridge::publish_node_status_v1,
};

use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

#[cfg(not(target_os = "windows"))]
use std::{
    fs,
    path::PathBuf,
};
use zeroize::Zeroizing;

#[cfg(not(target_os = "windows"))]
const SYSTEMD_WALLET_CREDENTIAL: &str =
    "edgeswarm-wallet-password";

#[cfg(not(target_os = "windows"))]
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

#[cfg(target_os = "windows")]
fn read_wallet_password() -> Result<Zeroizing<String>, String> {
    edgeswarm_unified_node_lib::core::
        windows_restart_credential::
        read_windows_restart_credential_v1()
}

#[cfg(not(target_os = "windows"))]
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

#[cfg(target_os = "windows")]
fn windows_desired_state_allows_start_v1() -> bool {
    let state =
        effective_desired_node_state_v1();

    let running =
        state.desired_state ==
        DesiredNodeStateV1::Running;

    println!(
        "HEADLESS_DESIRED_STATE={}",
        match state.desired_state {
            DesiredNodeStateV1::Running =>
                "running",
            DesiredNodeStateV1::UserStopped =>
                "user_stopped",
        }
    );

    println!(
        "HEADLESS_NODE_START_ALLOWED={running}"
    );

    running
}

#[cfg(target_os = "windows")]
fn start_status_publisher_v1(
    done: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !done.load(Ordering::Acquire) {
            let desired =
                effective_desired_node_state_v1();

            let stopping =
                desired.desired_state
                    == DesiredNodeStateV1::UserStopped;

            let _ =
                publish_node_status_v1(
                    true,
                    stopping,
                    None,
                );

            thread::sleep(
                Duration::from_millis(500)
            );
        }
    })
}

#[cfg(target_os = "windows")]
fn start_desired_state_monitor_v1(
    stop: Arc<AtomicBool>,
    monitor_done: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !monitor_done.load(Ordering::Acquire) {
            let state =
                effective_desired_node_state_v1();

            if state.desired_state ==
                DesiredNodeStateV1::UserStopped
            {
                println!(
                    "HEADLESS_USER_STOP_OBSERVED=true"
                );

                stop.store(
                    true,
                    Ordering::Release,
                );

                break;
            }

            thread::sleep(
                Duration::from_millis(500)
            );
        }

        println!(
            "HEADLESS_DESIRED_STATE_MONITOR_STOPPED=true"
        );
    })
}

fn run() -> Result<(), String> {
    // WINDOWS_HEADLESS_DESIRED_STATE_V1
    //
    // Fail closed before reading wallet material or starting any
    // runtime. A supervisor may launch this process, but only a
    // persisted user intent of `running` is allowed to start work.
    #[cfg(target_os = "windows")]
    if !windows_desired_state_allows_start_v1() {
        let _ =
            publish_node_status_v1(
                false,
                false,
                None,
            );

        println!(
            "HEADLESS_NODE_SUPERVISOR_RESTART_ALLOWED=false"
        );

        return Ok(());
    }

    let wallet_password =
        read_wallet_password()?;

    #[cfg(target_os = "windows")]
    println!(
        "HEADLESS_WINDOWS_DPAPI_CREDENTIAL_LOADED=true"
    );

    let stop =
        Arc::new(AtomicBool::new(false));

    let signal_stop =
        Arc::clone(&stop);

    ctrlc::set_handler(move || {
        signal_stop.store(
            true,
            Ordering::Release,
        );
    })
    .map_err(|_| {
        "shutdown_signal_handler_failed".to_string()
    })?;

    #[cfg(target_os = "windows")]
    let monitor_done =
        Arc::new(AtomicBool::new(false));

    #[cfg(target_os = "windows")]
    let desired_state_monitor =
        start_desired_state_monitor_v1(
            Arc::clone(&stop),
            Arc::clone(&monitor_done),
        );

    #[cfg(target_os = "windows")]
    let status_done =
        Arc::new(AtomicBool::new(false));

    #[cfg(target_os = "windows")]
    let status_publisher =
        start_status_publisher_v1(
            Arc::clone(&status_done),
        );

    let _power_guard =
        PowerGuard::acquire()?;

    println!("HEADLESS_NODE_MODE=true");
    println!(
        "HEADLESS_NODE_SIGNAL_HANDLER_READY=true"
    );
    println!(
        "HEADLESS_NODE_POWER_GUARD_READY=true"
    );

    let result =
        run_node_service(
            Arc::clone(&stop),
            wallet_password,
        );

    #[cfg(target_os = "windows")]
    {
        monitor_done.store(
            true,
            Ordering::Release,
        );

        let _ =
            desired_state_monitor.join();

        status_done.store(
            true,
            Ordering::Release,
        );

        let _ =
            status_publisher.join();

        let final_state =
            effective_desired_node_state_v1();

        if final_state.desired_state ==
            DesiredNodeStateV1::UserStopped
        {
            let _ =
                publish_node_status_v1(
                    false,
                    false,
                    None,
                );

            println!(
                "HEADLESS_NODE_EXIT_REASON=user_stopped"
            );

            println!(
                "HEADLESS_NODE_SUPERVISOR_RESTART_ALLOWED=false"
            );

            // Intentional STOP is a normal clean exit even if the
            // task lifecycle itself observed a shutdown condition.
            return Ok(());
        }

        let _ =
            publish_node_status_v1(
                false,
                false,
                result.as_ref()
                    .err()
                    .cloned(),
            );
    }

    result?;

    #[cfg(target_os = "windows")]
    {
        // Under supervision, a clean worker exit while the persisted
        // intent remains `running` is abnormal and should be restarted.
        if env::var("EDGESWARM_SUPERVISED")
            .map(|value| value.trim() == "1")
            .unwrap_or(false)
        {
            println!(
                "HEADLESS_NODE_EXIT_REASON=unexpected_clean_exit"
            );

            println!(
                "HEADLESS_NODE_SUPERVISOR_RESTART_ALLOWED=true"
            );

            return Err(
                "headless_unexpected_clean_exit_while_running"
                    .into()
            );
        }
    }

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        #[cfg(target_os = "windows")]
        {
            let _ =
                publish_node_status_v1(
                    false,
                    false,
                    Some(error.clone()),
                );
        }

        eprintln!(
            "HEADLESS_NODE_ERROR={}",
            error.replace('\n', " ")
        );

        let exit_code =
            if error ==
                "node_service_already_running"
            {
                73
            } else {
                1
            };

        std::process::exit(exit_code);
    }
}
