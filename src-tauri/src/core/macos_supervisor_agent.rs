#![cfg(target_os = "macos")]

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
};

const LABEL: &str =
    "com.edgeswarm.node.supervisor";

fn executable_dir() -> Result<PathBuf, String> {
    let exe =
        std::env::current_exe()
            .map_err(|_| {
                "macos_current_exe_failed".to_string()
            })?;

    exe.parent()
        .map(PathBuf::from)
        .ok_or_else(|| {
            "macos_executable_directory_missing"
                .to_string()
        })
}

fn headless_path() -> Result<PathBuf, String> {
    let path =
        executable_dir()?
            .join("edgeswarm-node-headless");

    if !path.is_file() {
        return Err(
            "macos_headless_helper_missing".into()
        );
    }

    Ok(path)
}

fn supervisor_path() -> Result<PathBuf, String> {
    let path =
        executable_dir()?
            .join(
                "edgeswarm-node-supervisor-macos"
            );

    if !path.is_file() {
        return Err(
            "macos_supervisor_helper_missing".into()
        );
    }

    Ok(path)
}

fn launch_agent_path() -> Result<PathBuf, String> {
    let home =
        std::env::var_os("HOME")
            .ok_or_else(|| {
                "macos_home_missing".to_string()
            })?;

    Ok(
        PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LABEL}.plist"))
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn persist_restart_credential_v1(
    password: &str,
) -> Result<(), String> {
    if password.is_empty() {
        return Err(
            "macos_restart_credential_empty".into()
        );
    }

    let mut child =
        Command::new(headless_path()?)
            .arg(
                "--persist-restart-credential"
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| {
                "macos_credential_helper_launch_failed"
                    .to_string()
            })?;

    {
        let mut stdin =
            child.stdin.take()
                .ok_or_else(|| {
                    "macos_credential_helper_stdin_missing"
                        .to_string()
                })?;

        stdin
            .write_all(password.as_bytes())
            .map_err(|_| {
                "macos_credential_helper_write_failed"
                    .to_string()
            })?;
    }

    let output =
        child.wait_with_output()
            .map_err(|_| {
                "macos_credential_helper_wait_failed"
                    .to_string()
            })?;

    if !output.status.success() {
        let error =
            String::from_utf8_lossy(
                &output.stderr
            )
            .trim()
            .replace('\n', " ");

        return Err(format!(
            "macos_credential_helper_failed:{error}"
        ));
    }

    Ok(())
}

pub fn ensure_launch_agent_v1(
) -> Result<(), String> {
    let supervisor =
        supervisor_path()?;

    let plist =
        launch_agent_path()?;

    let parent =
        plist.parent()
            .ok_or_else(|| {
                "macos_launch_agent_parent_missing"
                    .to_string()
            })?;

    fs::create_dir_all(parent)
        .map_err(|_| {
            "macos_launch_agent_directory_failed"
                .to_string()
        })?;

    let data =
        crate::adapters::app_data_dir();

    fs::create_dir_all(&data)
        .map_err(|_| {
            "macos_supervisor_log_directory_failed"
                .to_string()
        })?;

    let stdout =
        data.join("macos-supervisor.log");

    let stderr =
        data.join("macos-supervisor-error.log");

    let contents = format!(
r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>

    <key>ProgramArguments</key>
    <array>
        <string>{supervisor}</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>ProcessType</key>
    <string>Background</string>

    <key>ThrottleInterval</key>
    <integer>2</integer>

    <key>StandardOutPath</key>
    <string>{stdout}</string>

    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>
"#,
        label = LABEL,
        supervisor =
            xml_escape(
                &supervisor.to_string_lossy()
            ),
        stdout =
            xml_escape(
                &stdout.to_string_lossy()
            ),
        stderr =
            xml_escape(
                &stderr.to_string_lossy()
            ),
    );

    let temporary =
        plist.with_extension("tmp");

    fs::write(
        &temporary,
        contents.as_bytes(),
    )
    .map_err(|_| {
        "macos_launch_agent_write_failed"
            .to_string()
    })?;

    fs::set_permissions(
        &temporary,
        fs::Permissions::from_mode(0o644),
    )
    .map_err(|_| {
        "macos_launch_agent_permissions_failed"
            .to_string()
    })?;

    fs::rename(
        &temporary,
        &plist,
    )
    .map_err(|_| {
        "macos_launch_agent_commit_failed"
            .to_string()
    })?;

    let uid =
        unsafe { libc::getuid() };

    let domain =
        format!("gui/{uid}");

    let service =
        format!("{domain}/{LABEL}");

    let loaded =
        Command::new("/bin/launchctl")
            .args([
                "print",
                service.as_str(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);

    if loaded {
        println!(
            "MACOS_LAUNCH_AGENT_ALREADY_LOADED=true"
        );

        return Ok(());
    }

    let status =
        Command::new("/bin/launchctl")
            .args([
                "bootstrap",
                domain.as_str(),
                plist.to_string_lossy().as_ref(),
            ])
            .status()
            .map_err(|_| {
                "macos_launch_agent_bootstrap_failed"
                    .to_string()
            })?;

    if !status.success() {
        return Err(format!(
            "macos_launch_agent_bootstrap_exit_{}",
            status.code().unwrap_or(-1)
        ));
    }

    println!(
        "MACOS_LAUNCH_AGENT_BOOTSTRAPPED=true"
    );

    Ok(())
}


const UPDATE_PAUSE_MAX_AGE_MS: u64 =
    15 * 60 * 1000;

#[derive(
    Debug,
    serde::Serialize,
    serde::Deserialize
)]
#[serde(rename_all = "camelCase")]
struct MacosUpdatePauseV1 {
    schema_version: u8,
    updater_pid: u32,
    created_at_unix_ms: u64,
}

fn now_unix_ms_v1() -> u64 {
    std::time::SystemTime::now()
        .duration_since(
            std::time::UNIX_EPOCH
        )
        .map(|value| {
            value.as_millis() as u64
        })
        .unwrap_or(0)
}

fn update_pause_path_v1(
) -> std::path::PathBuf {
    crate::adapters::app_data_dir()
        .join("macos-update-pause.json")
}

fn process_alive_v1(
    pid: u32,
) -> bool {
    if pid == 0 {
        return false;
    }

    let result =
        unsafe {
            libc::kill(
                pid as i32,
                0,
            )
        };

    if result == 0 {
        return true;
    }

    std::io::Error::last_os_error()
        .raw_os_error()
        == Some(libc::EPERM)
}

pub fn begin_update_pause_v1(
) -> Result<(), String> {
    let path =
        update_pause_path_v1();

    if let Some(parent) =
        path.parent()
    {
        fs::create_dir_all(parent)
            .map_err(|_| {
                "macos_update_pause_directory_failed"
                    .to_string()
            })?;
    }

    let pause =
        MacosUpdatePauseV1 {
            schema_version: 1,
            updater_pid:
                std::process::id(),
            created_at_unix_ms:
                now_unix_ms_v1(),
        };

    let raw =
        serde_json::to_vec_pretty(
            &pause
        )
        .map_err(|_| {
            "macos_update_pause_serialize_failed"
                .to_string()
        })?;

    let temporary =
        path.with_extension(format!(
            "tmp-{}",
            std::process::id()
        ));

    fs::write(
        &temporary,
        raw,
    )
    .map_err(|_| {
        "macos_update_pause_write_failed"
            .to_string()
    })?;

    fs::rename(
        &temporary,
        &path,
    )
    .map_err(|_| {
        "macos_update_pause_commit_failed"
            .to_string()
    })?;

    println!(
        "MACOS_UPDATE_PAUSE_CREATED=true"
    );

    Ok(())
}

pub fn clear_update_pause_v1(
) -> Result<(), String> {
    let path =
        update_pause_path_v1();

    if path.exists() {
        fs::remove_file(&path)
            .map_err(|_| {
                "macos_update_pause_remove_failed"
                    .to_string()
            })?;
    }

    println!(
        "MACOS_UPDATE_PAUSE_CLEARED=true"
    );

    Ok(())
}

pub fn update_pause_active_v1(
) -> bool {
    let path =
        update_pause_path_v1();

    let Ok(raw) =
        fs::read_to_string(&path)
    else {
        return false;
    };

    let Ok(pause) =
        serde_json::from_str::<
            MacosUpdatePauseV1
        >(&raw)
    else {
        let _ =
            fs::remove_file(&path);

        println!(
            "MACOS_UPDATE_PAUSE_INVALID_CLEARED=true"
        );

        return false;
    };

    if pause.schema_version != 1 {
        let _ =
            fs::remove_file(&path);

        return false;
    }

    let age =
        now_unix_ms_v1()
            .saturating_sub(
                pause.created_at_unix_ms
            );

    if age > UPDATE_PAUSE_MAX_AGE_MS {
        let _ =
            fs::remove_file(&path);

        println!(
            "MACOS_UPDATE_PAUSE_STALE_CLEARED=true"
        );

        return false;
    }

    if !process_alive_v1(
        pause.updater_pid
    ) {
        let _ =
            fs::remove_file(&path);

        println!(
            "MACOS_UPDATE_PAUSE_ORPHAN_CLEARED=true"
        );

        return false;
    }

    true
}

fn process_pattern_active_v1(
    pattern: &str,
) -> bool {
    Command::new("/usr/bin/pgrep")
        .args([
            "-f",
            pattern,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn provider_processes_active_v1(
) -> bool {
    let headless =
        executable_dir()
            .ok()
            .map(|directory| {
                directory
                    .join("edgeswarm-node-headless")
                    .to_string_lossy()
                    .to_string()
            });

    let llama =
        executable_dir()
            .ok()
            .map(|directory| {
                directory
                    .join("runtime")
                    .join("current")
                    .join("llama-server")
                    .to_string_lossy()
                    .to_string()
            });

    headless
        .as_deref()
        .map(process_pattern_active_v1)
        .unwrap_or(false)
        ||
    llama
        .as_deref()
        .map(process_pattern_active_v1)
        .unwrap_or(false)
}

pub fn wait_for_provider_paused_v1(
    timeout:
        std::time::Duration,
) -> Result<(), String> {
    let started =
        std::time::Instant::now();

    while started.elapsed() < timeout {
        let processes_active =
            provider_processes_active_v1();

        let bridge_running =
            match
                crate::core::
                    node_status_bridge::
                    load_fresh_node_status_v1(
                        3_000
                    )
            {
                Ok(Some(status)) =>
                    status.running,

                Ok(None) =>
                    false,

                Err(_) =>
                    false,
            };

        if !processes_active
            && !bridge_running
        {
            println!(
                "MACOS_UPDATE_PROVIDER_PROCESSES_CLEAR=true"
            );

            println!(
                "MACOS_UPDATE_PROVIDER_PAUSED=true"
            );

            return Ok(());
        }

        std::thread::sleep(
            std::time::Duration::
                from_millis(250)
        );
    }

    Err(
        "macos_update_provider_pause_timeout"
            .into()
    )
}


pub fn kickstart_supervisor_v1(
) -> Result<(), String> {
    let uid =
        unsafe { libc::getuid() };

    let service =
        format!(
            "gui/{uid}/{LABEL}"
        );

    let status =
        Command::new(
            "/bin/launchctl"
        )
        .args([
            "kickstart",
            "-k",
            service.as_str(),
        ])
        .status()
        .map_err(|_| {
            "macos_supervisor_kickstart_failed"
                .to_string()
        })?;

    if !status.success() {
        return Err(format!(
            "macos_supervisor_kickstart_exit_{}",
            status.code().unwrap_or(-1)
        ));
    }

    println!(
        "MACOS_SUPERVISOR_KICKSTARTED=true"
    );

    Ok(())
}


const UPDATER_LABEL: &str =
    "com.edgeswarm.node.updater";

fn updater_launch_agent_path_v1(
) -> Result<PathBuf, String> {
    let home =
        std::env::var_os("HOME")
            .ok_or_else(|| {
                "macos_home_missing".to_string()
            })?;

    Ok(
        PathBuf::from(home)
            .join("Library")
            .join("LaunchAgents")
            .join(format!(
                "{UPDATER_LABEL}.plist"
            ))
    )
}

pub fn ensure_update_agent_v1(
) -> Result<(), String> {
    let app_executable =
        std::env::current_exe()
            .map_err(|_| {
                "macos_updater_current_exe_failed"
                    .to_string()
            })?;

    let plist =
        updater_launch_agent_path_v1()?;

    let parent =
        plist.parent()
            .ok_or_else(|| {
                "macos_updater_agent_parent_missing"
                    .to_string()
            })?;

    fs::create_dir_all(parent)
        .map_err(|_| {
            "macos_updater_agent_directory_failed"
                .to_string()
        })?;

    let data =
        crate::adapters::app_data_dir();

    fs::create_dir_all(&data)
        .map_err(|_| {
            "macos_updater_log_directory_failed"
                .to_string()
        })?;

    let stdout =
        data.join(
            "macos-updater.stdout.log"
        );

    let stderr =
        data.join(
            "macos-updater.stderr.log"
        );

    let contents = format!(
r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>

    <key>ProgramArguments</key>
    <array>
        <string>{app}</string>
        <string>--background-update-check</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>StartInterval</key>
    <integer>3600</integer>

    <key>ProcessType</key>
    <string>Background</string>

    <key>StandardOutPath</key>
    <string>{stdout}</string>

    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>
"#,
        label =
            UPDATER_LABEL,
        app =
            xml_escape(
                &app_executable
                    .to_string_lossy()
            ),
        stdout =
            xml_escape(
                &stdout.to_string_lossy()
            ),
        stderr =
            xml_escape(
                &stderr.to_string_lossy()
            ),
    );

    let temporary =
        plist.with_extension("tmp");

    fs::write(
        &temporary,
        contents.as_bytes(),
    )
    .map_err(|_| {
        "macos_updater_agent_write_failed"
            .to_string()
    })?;

    fs::set_permissions(
        &temporary,
        fs::Permissions::from_mode(0o644),
    )
    .map_err(|_| {
        "macos_updater_agent_permissions_failed"
            .to_string()
    })?;

    fs::rename(
        &temporary,
        &plist,
    )
    .map_err(|_| {
        "macos_updater_agent_commit_failed"
            .to_string()
    })?;

    let uid =
        unsafe { libc::getuid() };

    let domain =
        format!("gui/{uid}");

    let service =
        format!(
            "{domain}/{UPDATER_LABEL}"
        );

    let loaded =
        Command::new("/bin/launchctl")
            .args([
                "print",
                service.as_str(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| {
                status.success()
            })
            .unwrap_or(false);

    if loaded {
        println!(
            "MACOS_UPDATE_AGENT_ALREADY_LOADED=true"
        );

        return Ok(());
    }

    let status =
        Command::new("/bin/launchctl")
            .args([
                "bootstrap",
                domain.as_str(),
                plist
                    .to_string_lossy()
                    .as_ref(),
            ])
            .status()
            .map_err(|_| {
                "macos_updater_agent_bootstrap_failed"
                    .to_string()
            })?;

    if !status.success() {
        return Err(format!(
            "macos_updater_agent_bootstrap_exit_{}",
            status.code().unwrap_or(-1)
        ));
    }

    println!(
        "MACOS_UPDATE_AGENT_BOOTSTRAPPED=true"
    );

    Ok(())
}
