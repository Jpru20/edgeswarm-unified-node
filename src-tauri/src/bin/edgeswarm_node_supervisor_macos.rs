#[cfg(target_os = "macos")]
mod macos {
    use edgeswarm_unified_node_lib::core::{
        desired_state::{
            desired_state_path_v1,
            effective_desired_node_state_v1,
            DesiredNodeStateV1,
        },
        macos_supervisor_agent::
            update_pause_active_v1,
    };
    use fs2::FileExt;
    use std::{
        fs::{self, File, OpenOptions},
        os::unix::process::CommandExt,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    const POLL_INTERVAL: Duration =
        Duration::from_millis(500);

    const STOP_GRACE: Duration =
        Duration::from_secs(15);

    const STABLE_RUNTIME: Duration =
        Duration::from_secs(60);

    const RESTART_DELAYS: [u64; 6] =
        [1, 2, 5, 10, 20, 30];

    struct SupervisorLock {
        _file: File,
    }

    fn desired_running() -> bool {
        effective_desired_node_state_v1()
            .desired_state
            == DesiredNodeStateV1::Running
    }

    fn provider_should_run() -> bool {
        desired_running()
            && !update_pause_active_v1()
    }

    fn acquire_lock() -> Result<SupervisorLock, String> {
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

        let file =
            OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(
                    data.join(
                        "macos-node-supervisor.lock"
                    )
                )
                .map_err(|_| {
                    "supervisor_lock_open_failed".to_string()
                })?;

        file.try_lock_exclusive()
            .map_err(|_| {
                "supervisor_already_running".to_string()
            })?;

        Ok(SupervisorLock {
            _file: file,
        })
    }

    fn headless_path(
    ) -> Result<std::path::PathBuf, String> {
        let exe =
            std::env::current_exe()
                .map_err(|_| {
                    "supervisor_current_exe_failed".to_string()
                })?;

        let parent =
            exe.parent()
                .ok_or_else(|| {
                    "supervisor_binary_directory_missing"
                        .to_string()
                })?;

        let headless =
            parent.join("edgeswarm-node-headless");

        if !headless.is_file() {
            return Err(
                "supervisor_headless_binary_missing".into()
            );
        }

        Ok(headless)
    }

    fn worker_logs() -> Result<(File, File), String> {
        let data =
            desired_state_path_v1()
                .parent()
                .ok_or_else(|| {
                    "supervisor_data_dir_missing".to_string()
                })?
                .to_path_buf();

        fs::create_dir_all(&data)
            .map_err(|_| {
                "supervisor_log_directory_failed".to_string()
            })?;

        let path =
            data.join(
                "macos-headless-supervisor.log"
            );

        let stdout =
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|_| {
                    "supervisor_log_open_failed".to_string()
                })?;

        let stderr =
            stdout.try_clone()
                .map_err(|_| {
                    "supervisor_log_clone_failed".to_string()
                })?;

        Ok((stdout, stderr))
    }

    fn spawn_headless() -> Result<Child, String> {
        if !provider_should_run() {
            return Err(
                "supervisor_start_blocked_user_stopped"
                    .into()
            );
        }

        let (stdout, stderr) =
            worker_logs()?;

        println!(
            "SUPERVISOR_HEADLESS_SPAWN_REQUESTED=true"
        );

        let mut command =
            Command::new(
                headless_path()?
            );

        command
            .env(
                "EDGESWARM_SUPERVISED",
                "1",
            )
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));

        // Headless is the process-group leader. llama-server
        // inherits this group, giving the supervisor authority
        // over the complete worker generation.
        command.process_group(0);

        command
            .spawn()
            .map_err(|_| {
                "supervisor_headless_spawn_failed"
                    .to_string()
            })
    }


    fn signal_process_group(
        group_id: u32,
        signal: i32,
    ) {
        if group_id == 0 {
            return;
        }

        let _ =
            unsafe {
                libc::kill(
                    -(group_id as i32),
                    signal,
                )
            };
    }

    fn cleanup_process_group(
        group_id: u32,
    ) {
        if group_id == 0 {
            return;
        }

        println!(
            "SUPERVISOR_PROCESS_GROUP_CLEANUP_PGID={group_id}"
        );

        signal_process_group(
            group_id,
            libc::SIGTERM,
        );

        thread::sleep(
            Duration::from_millis(750)
        );

        signal_process_group(
            group_id,
            libc::SIGKILL,
        );
    }

    fn restart_delay(
        failures: usize,
    ) -> Duration {
        let index =
            failures
                .saturating_sub(1)
                .min(RESTART_DELAYS.len() - 1);

        Duration::from_secs(
            RESTART_DELAYS[index]
        )
    }

    fn wait_restart(
        delay: Duration,
    ) -> bool {
        println!(
            "SUPERVISOR_RESTART_DELAY_SEC={}",
            delay.as_secs()
        );

        let started =
            Instant::now();

        while started.elapsed() < delay {
            if !provider_should_run() {
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

    pub fn run() -> Result<(), String> {
        let _lock =
            acquire_lock()?;

        println!(
            "MACOS_NODE_SUPERVISOR_MODE=true"
        );
        println!(
            "SUPERVISOR_TARGET=edgeswarm-node-headless"
        );
        println!(
            "SUPERVISOR_GUI_TARGET=false"
        );

        let mut child:
            Option<Child> = None;

        let mut child_started:
            Option<Instant> = None;

        let mut failures:
            usize = 0;

        let mut stop_seen:
            Option<Instant> = None;

        loop {
            if !provider_should_run() {
                failures = 0;

                if let Some(worker) =
                    child.as_mut()
                {
                    if stop_seen.is_none() {
                        stop_seen =
                            Some(Instant::now());

                        println!(
                            "SUPERVISOR_GRACEFUL_STOP_WAIT=true"
                        );

                        println!(
                            "SUPERVISOR_PROVIDER_PAUSE_OBSERVED=true"
                        );
                    }

                    match worker.try_wait() {
                        Ok(Some(status)) => {
                            println!(
                                "SUPERVISOR_HEADLESS_EXITED_STATUS={status}"
                            );

                            let old_group_id =
                                worker.id();

                            cleanup_process_group(
                                old_group_id
                            );

                            child = None;
                            child_started = None;
                            stop_seen = None;
                        }

                        Ok(None) => {
                            if stop_seen
                                .map(|value| {
                                    value.elapsed()
                                        >= STOP_GRACE
                                })
                                .unwrap_or(false)
                            {
                                let group_id =
                                    worker.id();

                                cleanup_process_group(
                                    group_id
                                );

                                let _ =
                                    worker.wait();

                                println!(
                                    "SUPERVISOR_PROCESS_GROUP_FORCED_STOP=true"
                                );

                                child = None;
                                child_started = None;
                                stop_seen = None;
                            }
                        }

                        Err(_) => {
                            child = None;
                            child_started = None;
                            stop_seen = None;
                        }
                    }
                }

                thread::sleep(
                    POLL_INTERVAL
                );

                continue;
            }

            stop_seen = None;

            if let Some(worker) =
                child.as_mut()
            {
                match worker.try_wait() {
                    Ok(None) => {
                        thread::sleep(
                            POLL_INTERVAL
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

                        let old_group_id =
                            worker.id();

                        cleanup_process_group(
                            old_group_id
                        );

                        child = None;
                        child_started = None;

                        if runtime >= STABLE_RUNTIME {
                            failures = 0;
                        }

                        failures =
                            failures.saturating_add(1);

                        let _ =
                            wait_restart(
                                restart_delay(failures)
                            );

                        continue;
                    }

                    Err(_) => {
                        child = None;
                        child_started = None;

                        failures =
                            failures.saturating_add(1);

                        let _ =
                            wait_restart(
                                restart_delay(failures)
                            );

                        continue;
                    }
                }
            }

            match spawn_headless() {
                Ok(worker) => {
                    println!(
                        "SUPERVISOR_HEADLESS_STARTED_PID={}",
                        worker.id()
                    );

                    child =
                        Some(worker);

                    child_started =
                        Some(Instant::now());
                }

                Err(error) => {
                    println!(
                        "SUPERVISOR_SPAWN_ERROR={error}"
                    );

                    failures =
                        failures.saturating_add(1);

                    let _ =
                        wait_restart(
                            restart_delay(failures)
                        );
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) =
        macos::run()
    {
        eprintln!(
            "SUPERVISOR_ERROR={}",
            error.replace('\n', " ")
        );

        std::process::exit(
            if error == "supervisor_already_running" {
                73
            } else {
                1
            }
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "SUPERVISOR_ERROR=macos_only"
    );

    std::process::exit(1);
}
