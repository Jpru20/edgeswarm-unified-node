#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
use std::{
    ffi::{OsStr, OsString},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW_V1: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn argument_value_v1(
    name: &str,
) -> Result<OsString, String> {
    let mut args =
        std::env::args_os().skip(1);

    while let Some(arg) = args.next() {
        if arg.as_os_str() == OsStr::new(name) {
            return args
                .next()
                .ok_or_else(|| {
                    format!(
                        "updater_runner_argument_value_missing:{name}"
                    )
                });
        }
    }

    Err(format!(
        "updater_runner_argument_missing:{name}"
    ))
}

#[cfg(target_os = "windows")]
fn run_hidden_updater_v1() -> Result<i32, String> {
    let install_dir =
        PathBuf::from(
            argument_value_v1("--install-dir")?
        );

    if !install_dir.is_dir() {
        return Err(
            "updater_runner_install_dir_missing".into()
        );
    }

    let script =
        install_dir
            .join("resources")
            .join("windows")
            .join("supervisor-task.ps1");

    let app =
        install_dir
            .join("edgeswarm-unified-node.exe");

    if !script.is_file() {
        return Err(
            "updater_runner_script_missing".into()
        );
    }

    if !app.is_file() {
        return Err(
            "updater_runner_app_missing".into()
        );
    }

    let powershell =
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(r"C:\Windows")
            })
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");

    if !powershell.is_file() {
        return Err(
            "updater_runner_powershell_missing".into()
        );
    }

    let status =
        Command::new(powershell)
            .creation_flags(
                CREATE_NO_WINDOW_V1
            )
            .arg("-NoProfile")
            .arg("-WindowStyle")
            .arg("Hidden")
            .arg("-NonInteractive")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(script)
            .arg("-Mode")
            .arg("RunUpdater")
            .arg("-AppPath")
            .arg(app)
            .current_dir(&install_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| {
                "updater_runner_powershell_launch_failed"
                    .to_string()
            })?;

    Ok(status.code().unwrap_or(1))
}

#[cfg(target_os = "windows")]
fn main() {
    let code =
        match run_hidden_updater_v1() {
            Ok(value) => value,
            Err(_) => 1,
        };

    std::process::exit(code);
}

#[cfg(not(target_os = "windows"))]
fn main() {
    std::process::exit(1);
}