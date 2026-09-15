#[cfg(target_os = "windows")]
use std::{
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW_V1: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn installed_paths_v1(
) -> Result<(PathBuf, PathBuf), String> {
    let current =
        std::env::current_exe()
            .map_err(|_| {
                "supervisor_task_current_exe_failed"
                    .to_string()
            })?;

    let root =
        current.parent()
            .ok_or_else(|| {
                "supervisor_task_install_dir_missing"
                    .to_string()
            })?;

    let supervisor =
        std::env::var_os(
            "EDGESWARM_SUPERVISOR_PATH"
        )
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join(
                "edgeswarm-node-supervisor.exe"
            )
        });

    let script =
        std::env::var_os(
            "EDGESWARM_SUPERVISOR_TASK_SCRIPT"
        )
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("resources")
                .join("windows")
                .join("supervisor-task.ps1")
        });

    Ok((supervisor, script))
}

#[cfg(target_os = "windows")]
pub fn ensure_windows_supervisor_task_v1(
) -> Result<(), String> {
    let (supervisor, script) =
        installed_paths_v1()?;

    if !supervisor.is_file() {
        return Err(
            "windows_supervisor_binary_missing"
                .into()
        );
    }

    if !script.is_file() {
        return Err(
            "windows_supervisor_task_script_missing"
                .into()
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
            "windows_powershell_missing".into()
        );
    }

    let status =
        Command::new(powershell)
            .creation_flags(
                CREATE_NO_WINDOW_V1
            )
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&script)
            .arg("-Mode")
            .arg("Install")
            .arg("-SupervisorPath")
            .arg(&supervisor)
            .status()
            .map_err(|_| {
                "windows_supervisor_task_launch_failed"
                    .to_string()
            })?;

    if !status.success() {
        return Err(format!(
            "windows_supervisor_task_install_exit_{}",
            status.code().unwrap_or(-1)
        ));
    }

    println!(
        "WINDOWS_SUPERVISOR_TASK_ENSURED=true"
    );

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_windows_supervisor_task_v1(
) -> Result<(), String> {
    Ok(())
}
