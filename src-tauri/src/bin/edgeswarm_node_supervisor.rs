#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(target_os = "windows")]
use edgeswarm_unified_node_lib::core::desired_state::{
    desired_state_path_v1,
    effective_desired_node_state_v1,
    DesiredNodeStateV1,
};

#[cfg(target_os = "windows")]
use fs2::FileExt;

#[cfg(target_os = "windows")]
use std::{
    fs::{self, File, OpenOptions},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{
        CloseHandle,
        GetLastError,
        HANDLE,
        INVALID_HANDLE_VALUE,
    },
    System::{
        JobObjects::{
            AssignProcessToJobObject,
            CreateJobObjectW,
            JobObjectExtendedLimitInformation,
            SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::GetCurrentProcess,
    },
};

#[cfg(target_os = "windows")]
const POLL_INTERVAL_V1: Duration =
    Duration::from_millis(500);

#[cfg(target_os = "windows")]
const STOP_GRACE_V1: Duration =
    Duration::from_secs(15);

#[cfg(target_os = "windows")]
const STABLE_RUNTIME_V1: Duration =
    Duration::from_secs(60);

#[cfg(target_os = "windows")]
const UPDATE_PAUSE_STALE_V1: Duration =
    Duration::from_secs(30 * 60);

#[cfg(target_os = "windows")]
const RESTART_DELAYS_SEC_V1: [u64; 6] =
    [1, 2, 5, 10, 20, 30];

#[cfg(target_os = "windows")]
struct SupervisorJobV1 {
    // Intentionally kept open for the entire process lifetime.
    // Windows closes it during supervisor process teardown.
    _process_lifetime_handle: HANDLE,
}

#[cfg(target_os = "windows")]
fn job_win32_error_v1(
    operation: &str,
) -> String {
    let code =
        unsafe { GetLastError() };

    format!(
        "{operation}_failed_win32_{code}"
    )
}

#[cfg(target_os = "windows")]
fn establish_supervisor_job_v1(
) -> Result<SupervisorJobV1, String> {
    let handle =
        unsafe {
            CreateJobObjectW(
                std::ptr::null(),
                std::ptr::null(),
            )
        };

    if handle.is_null()
        || handle == INVALID_HANDLE_VALUE
    {
        return Err(
            job_win32_error_v1(
                "supervisor_job_create"
            )
        );
    }

    let mut information:
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION =
        unsafe { std::mem::zeroed() };

    information
        .BasicLimitInformation
        .LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

    let configured =
        unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (
                    &information
                    as *const
                        JOBOBJECT_EXTENDED_LIMIT_INFORMATION
                ).cast(),
                std::mem::size_of::<
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION
                >() as u32,
            )
        };

    if configured == 0 {
        let error =
            job_win32_error_v1(
                "supervisor_job_configure"
            );

        unsafe {
            let _ = CloseHandle(handle);
        }

        return Err(error);
    }

    let assigned =
        unsafe {
            AssignProcessToJobObject(
                handle,
                GetCurrentProcess(),
            )
        };

    if assigned == 0 {
        let error =
            job_win32_error_v1(
                "supervisor_job_assign_self"
            );

        unsafe {
            let _ = CloseHandle(handle);
        }

        return Err(error);
    }

    println!(
        "SUPERVISOR_JOB_OBJECT_ACTIVE=true"
    );

    println!(
        "SUPERVISOR_JOB_KILL_ON_CLOSE=true"
    );

    println!(
        "SUPERVISOR_JOB_CHILD_INHERITANCE=true"
    );

    Ok(SupervisorJobV1 {
        _process_lifetime_handle: handle,
    })
}

#[cfg(target_os = "windows")]
struct SupervisorLockV1 {
    _file: File,
}

#[cfg(target_os = "windows")]
fn acquire_supervisor_lock_v1(
) -> Result<SupervisorLockV1, String> {
    let data =
        desired_state_path_v1()
            .parent()
            .ok_or_else(|| {
                "supervisor_data_dir_missing".to_string()
            })?
            .to_path_buf();

    fs::create_dir_all(&data)
        .map_err(|_| {
            "supervisor_data_dir_failed".to_string()
        })?;

    let path =
        data.join("node-supervisor.lock");

    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| {
            "supervisor_lock_open_failed".to_string()
        })?;

    file.try_lock_exclusive()
        .map_err(|_| {
            "supervisor_already_running".to_string()
        })?;

    Ok(SupervisorLockV1 {
        _file: file,
    })
}

#[cfg(target_os = "windows")]
fn desired_running_v1() -> bool {
    effective_desired_node_state_v1()
        .desired_state
        == DesiredNodeStateV1::Running
}

#[cfg(target_os = "windows")]
fn headless_path_v1(
) -> Result<std::path::PathBuf, String> {
    let current =
        std::env::current_exe()
            .map_err(|_| {
                "supervisor_current_exe_failed"
                    .to_string()
            })?;

    let parent =
        current.parent()
            .ok_or_else(|| {
                "supervisor_binary_directory_missing"
                    .to_string()
            })?;

    let headless =
        parent.join("edgeswarm-node-headless.exe");

    if !headless.is_file() {
        return Err(
            "supervisor_headless_binary_missing".into()
        );
    }

    Ok(headless)
}

#[cfg(target_os = "windows")]
fn worker_log_v1(
) -> Result<(File, File), String> {
    let data =
        desired_state_path_v1()
            .parent()
            .ok_or_else(|| {
                "supervisor_data_dir_missing".to_string()
            })?
            .to_path_buf();

    fs::create_dir_all(&data)
        .map_err(|_| {
            "supervisor_log_directory_failed"
                .to_string()
        })?;

    let path =
        data.join("headless-supervisor.log");

    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|_| {
            "supervisor_log_open_failed".to_string()
        })?;

    let stderr =
        stdout.try_clone()
            .map_err(|_| {
                "supervisor_log_clone_failed"
                    .to_string()
            })?;

    Ok((stdout, stderr))
}

#[cfg(target_os = "windows")]
fn spawn_headless_v1(
) -> Result<Child, String> {
    if !desired_running_v1() {
        return Err(
            "supervisor_start_blocked_user_stopped"
                .into()
        );
    }

    let headless =
        headless_path_v1()?;

    let (stdout, stderr) =
        worker_log_v1()?;

    println!(
        "SUPERVISOR_HEADLESS_SPAWN_REQUESTED=true"
    );

    Command::new(headless)
        .env(
            "EDGESWARM_SUPERVISED",
            "1",
        )
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|_| {
            "supervisor_headless_spawn_failed"
                .to_string()
        })
}

#[cfg(target_os = "windows")]
fn restart_delay_v1(
    failures: usize,
) -> Duration {
    let index =
        failures
            .saturating_sub(1)
            .min(
                RESTART_DELAYS_SEC_V1.len() - 1
            );

    Duration::from_secs(
        RESTART_DELAYS_SEC_V1[index]
    )
}

#[cfg(target_os = "windows")]
fn wait_restart_delay_v1(
    delay: Duration,
) -> bool {
    println!(
        "SUPERVISOR_RESTART_DELAY_SEC={}",
        delay.as_secs()
    );

    let started =
        Instant::now();

    while started.elapsed() < delay {
        if !desired_running_v1() {
            println!(
                "SUPERVISOR_RESTART_CANCELLED_USER_STOP=true"
            );

            return false;
        }

        thread::sleep(
            Duration::from_millis(250)
        );
    }

    true
}

#[cfg(target_os = "windows")]
fn update_pause_active_v1(
) -> Result<bool, String> {
    // SUPERVISOR_UPDATE_PAUSE_LOCK_V1
    let exe =
        std::env::current_exe()
            .map_err(|_| {
                "supervisor_current_exe_failed".to_string()
            })?;

    let root =
        exe.parent()
            .ok_or_else(|| {
                "supervisor_install_dir_missing".to_string()
            })?;

    let pause_path =
        root.join("update-pause.lock");

    if !pause_path.is_file() {
        return Ok(false);
    }

    if let Ok(metadata) =
        fs::metadata(&pause_path)
    {
        if let Ok(modified) =
            metadata.modified()
        {
            if modified
                .elapsed()
                .map(|age| {
                    age >= UPDATE_PAUSE_STALE_V1
                })
                .unwrap_or(false)
            {
                let _ =
                    fs::remove_file(&pause_path);

                println!(
                    "SUPERVISOR_UPDATE_PAUSE_STALE_CLEARED=true"
                );

                return Ok(false);
            }
        }
    }

    println!(
        "SUPERVISOR_UPDATE_PAUSED=true"
    );

    Ok(true)
}

#[cfg(target_os = "windows")]
fn run_supervisor_v1(
) -> Result<(), String> {
    if update_pause_active_v1()? {
        return Ok(());
    }
    let _instance =
        acquire_supervisor_lock_v1()?;

    // WINDOWS_SUPERVISOR_JOB_OBJECT_V1
    //
    // Put the supervisor itself in a kill-on-close Job Object.
    // Headless and llama descendants therefore remain in the same
    // process tree automatically and cannot survive supervisor loss.
    let _job =
        establish_supervisor_job_v1()?;

    println!(
        "WINDOWS_NODE_SUPERVISOR_MODE=true"
    );
    println!(
        "SUPERVISOR_TARGET=edgeswarm-node-headless.exe"
    );
    println!(
        "SUPERVISOR_GUI_TARGET=false"
    );

    let mut child: Option<Child> =
        None;

    let mut child_started:
        Option<Instant> = None;

    let mut failure_count:
        usize = 0;

    let mut stop_observed:
        Option<Instant> = None;

    loop {
        let running =
            desired_running_v1();

        if !running {
            failure_count = 0;

            if let Some(worker) =
                child.as_mut()
            {
                if stop_observed.is_none() {
                    stop_observed =
                        Some(Instant::now());

                    println!(
                        "SUPERVISOR_USER_STOP_OBSERVED=true"
                    );
                }

                match worker.try_wait() {
                    Ok(Some(status)) => {
                        println!(
                            "SUPERVISOR_HEADLESS_EXITED_STATUS={status}"
                        );

                        child = None;
                        child_started = None;
                        stop_observed = None;
                    }

                    Ok(None) => {
                        if stop_observed
                            .map(|value| {
                                value.elapsed()
                                    >= STOP_GRACE_V1
                            })
                            .unwrap_or(false)
                        {
                            println!(
                                "SUPERVISOR_STOP_GRACE_EXPIRED=true"
                            );

                            let _ =
                                worker.kill();

                            let _ =
                                worker.wait();

                            println!(
                                "SUPERVISOR_HEADLESS_FORCED_STOP=true"
                            );

                            child = None;
                            child_started = None;
                            stop_observed = None;
                        }
                    }

                    Err(_) => {
                        child = None;
                        child_started = None;
                        stop_observed = None;
                    }
                }
            } else {
                stop_observed = None;
            }

            thread::sleep(
                POLL_INTERVAL_V1
            );

            continue;
        }

        stop_observed = None;

        if let Some(worker) =
            child.as_mut()
        {
            match worker.try_wait() {
                Ok(None) => {
                    thread::sleep(
                        POLL_INTERVAL_V1
                    );

                    continue;
                }

                Ok(Some(status)) => {
                    let runtime =
                        child_started
                            .map(|value| {
                                value.elapsed()
                            })
                            .unwrap_or_default();

                    println!(
                        "SUPERVISOR_HEADLESS_EXITED_STATUS={status}"
                    );

                    child = None;
                    child_started = None;

                    if runtime >=
                        STABLE_RUNTIME_V1
                    {
                        failure_count = 0;
                    }

                    failure_count =
                        failure_count
                            .saturating_add(1);

                    let delay =
                        restart_delay_v1(
                            failure_count
                        );

                    if wait_restart_delay_v1(
                        delay
                    ) {
                        continue;
                    }

                    continue;
                }

                Err(_) => {
                    child = None;
                    child_started = None;

                    failure_count =
                        failure_count
                            .saturating_add(1);

                    let delay =
                        restart_delay_v1(
                            failure_count
                        );

                    let _ =
                        wait_restart_delay_v1(
                            delay
                        );

                    continue;
                }
            }
        }

        match spawn_headless_v1() {
            Ok(worker) => {
                println!(
                    "SUPERVISOR_HEADLESS_STARTED_PID={}",
                    worker.id()
                );

                child = Some(worker);
                child_started =
                    Some(Instant::now());
            }

            Err(error) => {
                println!(
                    "SUPERVISOR_SPAWN_ERROR={error}"
                );

                failure_count =
                    failure_count
                        .saturating_add(1);

                let delay =
                    restart_delay_v1(
                        failure_count
                    );

                let _ =
                    wait_restart_delay_v1(
                        delay
                    );
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) =
        run_supervisor_v1()
    {
        eprintln!(
            "SUPERVISOR_ERROR={}",
            error.replace('\n', " ")
        );

        let code =
            if error ==
                "supervisor_already_running"
            {
                73
            } else {
                1
            };

        std::process::exit(code);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!(
        "SUPERVISOR_ERROR=windows_only"
    );

    std::process::exit(1);
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;

    #[test]
    fn restart_backoff_is_bounded_v1() {
        assert_eq!(
            restart_delay_v1(1),
            Duration::from_secs(1)
        );

        assert_eq!(
            restart_delay_v1(2),
            Duration::from_secs(2)
        );

        assert_eq!(
            restart_delay_v1(3),
            Duration::from_secs(5)
        );

        assert_eq!(
            restart_delay_v1(99),
            Duration::from_secs(30)
        );
    }
}
