#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::{Child, Command, Stdio};

#[cfg(target_os = "linux")]
use std::{
    io::Read,
    thread,
    time::Duration,
};

#[cfg(target_os = "windows")]
const ES_CONTINUOUS: u32 = 0x80000000;

#[cfg(target_os = "windows")]
const ES_SYSTEM_REQUIRED: u32 = 0x00000001;

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetThreadExecutionState(es_flags: u32) -> u32;
}

pub struct PowerGuard {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    child: Option<Child>,

    #[cfg(target_os = "windows")]
    windows_active: bool,
}

impl PowerGuard {
    pub fn acquire() -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let pid = std::process::id().to_string();

            let child = Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", &pid])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| {
                    format!("macos_sleep_inhibitor_start_failed:{error}")
                })?;

            println!("POWER_GUARD_ACQUIRED=true");
            println!("POWER_GUARD_PLATFORM=macos");
            println!("POWER_GUARD_MODE=prevent_idle_system_sleep");

            return Ok(Self {
                child: Some(child),
            });
        }

        #[cfg(target_os = "windows")]
        {
            let result = unsafe {
                SetThreadExecutionState(
                    ES_CONTINUOUS | ES_SYSTEM_REQUIRED
                )
            };

            if result == 0 {
                return Err(
                    "windows_sleep_inhibitor_start_failed".into()
                );
            }

            println!("POWER_GUARD_ACQUIRED=true");
            println!("POWER_GUARD_PLATFORM=windows");
            println!("POWER_GUARD_MODE=prevent_idle_system_sleep");

            return Ok(Self {
                windows_active: true,
            });
        }

        #[cfg(target_os = "linux")]
        {
            let desktop_session =
                std::env::var_os("DISPLAY").is_some() ||
                std::env::var_os("WAYLAND_DISPLAY").is_some();

            if !desktop_session {
                println!("POWER_GUARD_ACQUIRED=true");
                println!("POWER_GUARD_PLATFORM=linux");
                println!("POWER_GUARD_MODE=headless_server");
                println!("POWER_GUARD_INHIBITOR_REQUIRED=false");
                println!("POWER_GUARD_INHIBITOR_ACTIVE=false");

                return Ok(Self {
                    child: None,
                });
            }

            let mut child = Command::new("systemd-inhibit")
                .args([
                    "--what=sleep",
                    "--who=EdgeSwarm Node",
                    "--why=EdgeSwarm provider node is running",
                    "--mode=block",
                    "/bin/sleep",
                    "infinity",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|error| {
                    format!("linux_sleep_inhibitor_start_failed:{error}")
                })?;

            thread::sleep(Duration::from_millis(300));

            if let Some(status) = child
                .try_wait()
                .map_err(|error| {
                    format!("linux_sleep_inhibitor_status_failed:{error}")
                })?
            {
                let mut detail = String::new();

                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut detail);
                }

                let detail = detail.trim().replace('\n', " ");

                return Err(format!(
                    "linux_sleep_inhibitor_start_failed:status={status}:{}",
                    if detail.is_empty() {
                        "no_error_detail"
                    } else {
                        detail.as_str()
                    }
                ));
            }

            println!("POWER_GUARD_ACQUIRED=true");
            println!("POWER_GUARD_PLATFORM=linux");
            println!("POWER_GUARD_MODE=prevent_system_sleep");
            println!("POWER_GUARD_INHIBITOR_REQUIRED=true");
            println!("POWER_GUARD_INHIBITOR_ACTIVE=true");

            return Ok(Self {
                child: Some(child),
            });
        }

        #[allow(unreachable_code)]
        Err("sleep_inhibitor_unsupported_platform".into())
    }
}

impl Drop for PowerGuard {
    fn drop(&mut self) {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }

            println!("POWER_GUARD_RELEASED=true");
        }

        #[cfg(target_os = "windows")]
        {
            if self.windows_active {
                unsafe {
                    SetThreadExecutionState(ES_CONTINUOUS);
                }

                self.windows_active = false;
                println!("POWER_GUARD_RELEASED=true");
            }
        }
    }
}
