mod adapters;
pub mod core;
pub mod runtime;

#[cfg(feature = "desktop")]
mod desktop {
    use tauri::Manager;
    use crate::core::{
        auth_client::SupabaseAuthClient,
        auth_login_client::SupabaseLoginClient,
        auth_login_contract::{jwt_aal, verified_totp_factor},
        auth_session::AuthSession,
        desired_state::{persist_desired_node_state_v1, DesiredNodeStateV1},
        model_provisioning::{model_download_progress_v1, ModelDownloadProgressV1},
        node_service::{clear_node_service_logs, node_service_logs, run_node_service},
        wallet_bootstrap::bootstrap_authenticated_device_wallet_v1,
        NodeState,
    };
    use reqwest::blocking::Client;
    use serde::{Deserialize, Serialize};
    use std::{
        env,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use zeroize::Zeroizing;

    #[derive(Default)]
    struct AppAuthState {
        pending_email: Option<String>,
        pending_access_token: Option<Zeroizing<String>>,
        pending_factor_id: Option<String>,
        pending_challenge_id: Option<String>,

        authenticated_email: Option<String>,

        // Kept only in Rust process memory so the future node service can
        // unlock the encrypted device wallet without prompting in a terminal.
        wallet_password: Option<Zeroizing<String>>,
    }

    struct NodeRuntimeState {
        stop: Arc<AtomicBool>,
        running: Arc<AtomicBool>,
        last_error: Arc<Mutex<Option<String>>>,
        worker: Mutex<Option<JoinHandle<()>>>,
    }

    impl Default for NodeRuntimeState {
        fn default() -> Self {
            Self {
                stop: Arc::new(AtomicBool::new(false)),
                running: Arc::new(AtomicBool::new(false)),
                last_error: Arc::new(Mutex::new(None)),
                worker: Mutex::new(None),
            }
        }
    }

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct NodeServiceStatus {
        running: bool,
        desired_running: bool,
        stopping: bool,
        last_error: Option<String>,
        logs: Vec<String>,
        model_download: Option<ModelDownloadProgressV1>,
        certification: crate::core::certification_progress::CertificationProgressV1,
        capacity_test_requested: bool,
    }

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct AuthBeginResult {
        email: String,
        mfa_required: bool,
    }

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct AuthVerifyResult {
        email: String,
    }

    // PROVIDER_LEDGER_SYNC_COMMAND_V1
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ProviderLedgerApiResponse {
        total_earned_usd: f64,
        synced_at: Option<String>,
    }

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ProviderLedgerSummary {
        total_earned_usd: f64,
        synced_at: Option<String>,
    }

    fn provider_api_base_v1() -> String {
        env::var("GCP_BASE_URL")
            .unwrap_or_else(|_| "https://api.edgeswarm.io".into())
            .trim_end_matches('/')
            .to_string()
    }

    fn fetch_provider_ledger_v1(access_token: &str) -> Result<ProviderLedgerApiResponse, String> {
        let response = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| "provider_ledger_http_client_failed".to_string())?
            .get(format!("{}/v1/provider/ledger/me", provider_api_base_v1()))
            .bearer_auth(access_token)
            .send()
            .map_err(|_| "provider_ledger_network_failed".to_string())?;

        let status = response.status().as_u16();

        if !(200..300).contains(&status) {
            return Err(format!("provider_ledger_http_{status}"));
        }

        response
            .json::<ProviderLedgerApiResponse>()
            .map_err(|_| "provider_ledger_response_invalid".to_string())
    }

    #[tauri::command]
    fn provider_ledger_sync() -> Result<ProviderLedgerSummary, String> {
        let auth_client = SupabaseAuthClient::from_env()?;
        let ensured = auth_client.ensure_valid_session(true)?;

        let access_token = ensured
            .session
            .access_token()
            .ok_or_else(|| "provider_ledger_access_token_missing".to_string())?;

        let response = match fetch_provider_ledger_v1(access_token) {
            Err(error) if error == "provider_ledger_http_401" => {
                let refreshed = auth_client.force_refresh_session()?;
                let token = refreshed
                    .session
                    .access_token()
                    .ok_or_else(|| "provider_ledger_refreshed_token_missing".to_string())?;

                fetch_provider_ledger_v1(token)?
            }
            Err(error) => return Err(error),
            Ok(response) => response,
        };

        if !response.total_earned_usd.is_finite() || response.total_earned_usd < 0.0 {
            return Err("provider_ledger_usd_invalid".into());
        }

        Ok(ProviderLedgerSummary {
            total_earned_usd: response.total_earned_usd,
            synced_at: response.synced_at,
        })
    }

    #[tauri::command]
    fn set_window_layout(window: tauri::Window, screen: String) -> Result<(), String> {
        let (width, height) = if screen == "dashboard" {
            if cfg!(any(target_os = "macos", target_os = "linux")) {
                (560.0, 680.0)
            } else {
                (560.0, 560.0)
            }
        } else {
            (600.0, 360.0)
        };

        window
            .set_size(tauri::Size::Logical(tauri::LogicalSize::new(width, height)))
            .map_err(|error| format!("window_resize_failed:{error}"))
    }
    #[tauri::command]
    fn get_node_state() -> NodeState {
        NodeState::detect()
    }

    #[tauri::command]
    fn auth_begin(
        email: String,
        password: String,
        auth_state: tauri::State<'_, Mutex<AppAuthState>>,
    ) -> Result<AuthBeginResult, String> {
        let email = email.trim().to_lowercase();

        if email.is_empty() || password.is_empty() {
            return Err("email_or_password_missing".into());
        }

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

        let mut state = auth_state
            .lock()
            .map_err(|_| "auth_state_lock_failed".to_string())?;

        state.pending_email = Some(authenticated_email.clone());
        state.pending_access_token = Some(Zeroizing::new(aal1.access_token));
        state.pending_factor_id = Some(factor_id);
        state.pending_challenge_id = Some(challenge.id);

        // Preserve the login password only inside Rust memory.
        // The existing wallet encryption flow uses this password to unlock
        // the device wallet after successful MFA.
        state.wallet_password = Some(Zeroizing::new(password));

        Ok(AuthBeginResult {
            email: authenticated_email,
            mfa_required: true,
        })
    }

    #[tauri::command]
    async fn auth_verify(
        code: String,
        auth_state: tauri::State<'_, Mutex<AppAuthState>>,
    ) -> Result<AuthVerifyResult, String> {
        let code = code.trim().to_string();

        if code.len() != 6
            || !code.chars().all(|c| c.is_ascii_digit())
        {
            return Err(
                "invalid_mfa_code_format".into()
            );
        }

        // Copy only owned values while holding the lock.
        // No mutex guard is held across await or network work.
        let (
            email,
            access_token,
            factor_id,
            challenge_id,
            wallet_password,
        ) = {
            let state = auth_state
                .lock()
                .map_err(|_| {
                    "auth_state_lock_failed".to_string()
                })?;

            (
                state.pending_email.clone(),
                state.pending_access_token
                    .as_ref()
                    .map(|value| value.to_string()),
                state.pending_factor_id.clone(),
                state.pending_challenge_id.clone(),
                state.wallet_password
                    .as_ref()
                    .map(|value| {
                        Zeroizing::new(
                            value.as_str().to_owned()
                        )
                    }),
            )
        };

        let email = email.ok_or_else(|| {
            "pending_auth_email_missing".to_string()
        })?;

        let access_token =
            access_token.ok_or_else(|| {
                "pending_auth_access_token_missing"
                    .to_string()
            })?;

        let factor_id =
            factor_id.ok_or_else(|| {
                "pending_auth_factor_missing".to_string()
            })?;

        let challenge_id =
            challenge_id.ok_or_else(|| {
                "pending_auth_challenge_missing"
                    .to_string()
            })?;

        let wallet_password =
            wallet_password.ok_or_else(|| {
                "wallet_bootstrap_password_missing"
                    .to_string()
            })?;

        println!(
            "AUTH_VERIFY_BACKGROUND_WORKER_STARTED=true"
        );

        let verified_email =
            tauri::async_runtime::spawn_blocking(
                move || -> Result<String, String> {
                    println!(
                        "AUTH_VERIFY_STAGE=mfa_verify"
                    );

                    let login =
                        SupabaseLoginClient::from_env()?;

                    let verified =
                        login.verify(
                            &access_token,
                            &factor_id,
                            &challenge_id,
                            &code,
                        )?;

                    if jwt_aal(
                        &verified.access_token
                    ).as_deref()
                        != Some("aal2")
                    {
                        return Err(
                            "mfa_session_not_aal2"
                                .into()
                        );
                    }

                    let now =
                        SystemTime::now()
                            .duration_since(
                                UNIX_EPOCH
                            )
                            .map_err(|_| {
                                "clock_failed"
                                    .to_string()
                            })?
                            .as_secs();

                    let expires_at =
                        verified
                            .expires_at
                            .or_else(|| {
                                verified
                                    .expires_in
                                    .map(|seconds| {
                                        now.saturating_add(
                                            seconds
                                        )
                                    })
                            })
                            .ok_or_else(|| {
                                "mfa_session_expiry_missing"
                                    .to_string()
                            })?;

                    let session =
                        AuthSession::
                            from_authenticated_session(
                                &email,
                                &verified.access_token,
                                &verified.refresh_token,
                                expires_at,
                            )?;

                    println!(
                        "AUTH_VERIFY_STAGE=wallet_bootstrap"
                    );

                    bootstrap_authenticated_device_wallet_v1(
                        &email,
                        &verified.access_token,
                        wallet_password.as_str(),
                    )?;

                    println!(
                        "AUTH_VERIFY_STAGE=session_save"
                    );

                    session.save_secure()?;

                    println!(
                        "AUTH_VERIFY_STAGE=complete"
                    );

                    Ok(email)
                },
            )
            .await
            .map_err(|error| {
                format!(
                    "auth_verify_worker_join_failed:{error}"
                )
            })??;

        {
            let mut state = auth_state
                .lock()
                .map_err(|_| {
                    "auth_state_lock_failed".to_string()
                })?;

            if state.pending_email.as_deref()
                != Some(
                    verified_email.as_str()
                )
            {
                return Err(
                    "auth_state_changed_during_verify"
                        .into()
                );
            }

            state.pending_email = None;
            state.pending_access_token = None;
            state.pending_factor_id = None;
            state.pending_challenge_id = None;

            state.authenticated_email =
                Some(verified_email.clone());

            // wallet_password intentionally remains until
            // logout/app exit or until explicitly zeroized.
        }

        println!(
            "AUTH_VERIFY_BACKGROUND_WORKER_COMPLETE=true"
        );

        Ok(AuthVerifyResult {
            email: verified_email,
        })
    }

    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos"
    )))]
    fn current_node_service_status(
        runtime: &NodeRuntimeState,
    ) -> Result<NodeServiceStatus, String> {
        let running = runtime.running.load(Ordering::Acquire);
        let stopping = running && runtime.stop.load(Ordering::Acquire);

        let last_error = runtime
            .last_error
            .lock()
            .map_err(|_| "node_service_error_lock_failed".to_string())?
            .clone();

        let logs = node_service_logs();

        Ok(NodeServiceStatus {
            running,
            desired_running: running,
            stopping,
            last_error,
            logs,
            model_download: model_download_progress_v1(),
            certification: crate::core::certification_progress::certification_progress_v1(),
            capacity_test_requested:
                crate::core::capacity_test_control::
                    capacity_test_request_pending_v1(),
        })
    }


    #[cfg(any(
        target_os = "windows",
        target_os = "macos"
    ))]
    fn current_node_service_status(
        _runtime: &NodeRuntimeState,
    ) -> Result<NodeServiceStatus, String> {
        let desired =
            crate::core::desired_state::
                effective_desired_node_state_v1();

        let desired_running =
            desired.desired_state ==
                DesiredNodeStateV1::Running;

        let bridge =
            crate::core::node_status_bridge::
                load_fresh_node_status_v1(3_000)
                    .unwrap_or(None);

        let running =
            bridge.as_ref()
                .map(|value| value.running)
                .unwrap_or(false);

        let stopping =
            bridge.as_ref()
                .map(|value| {
                    value.stopping ||
                        (!desired_running && value.running)
                })
                .unwrap_or(false);

        Ok(NodeServiceStatus {
            running,
            desired_running,
            stopping,
            last_error:
                bridge.as_ref()
                    .and_then(|value| value.last_error.clone()),
            logs:
                bridge.as_ref()
                    .map(|value| value.logs.clone())
                    .unwrap_or_default(),
            model_download:
                bridge.as_ref()
                    .and_then(|value| value.model_download.clone()),
            certification:
                bridge.as_ref()
                    .map(|value| value.certification.clone())
                    .unwrap_or_default(),
            capacity_test_requested:
                bridge.as_ref()
                    .map(|value| {
                        value.capacity_test_requested
                    })
                    .unwrap_or_else(|| {
                        crate::core::capacity_test_control::
                            capacity_test_request_pending_v1()
                    }),
        })
    }

    #[tauri::command]
    fn request_capacity_test(
        runtime: tauri::State<'_, NodeRuntimeState>,
    ) -> Result<bool, String> {
        let status =
            current_node_service_status(&runtime)?;

        if !status.running ||
            !status.desired_running
        {
            return Err(
                "capacity_test_requires_running_node"
                    .into()
            );
        }

        if status.certification.state == "running" {
            return Err(
                "capacity_test_already_running"
                    .into()
            );
        }

        crate::core::capacity_test_control::
            request_capacity_test_v1()
    }

    #[tauri::command]
    fn node_service_status(
        runtime: tauri::State<'_, NodeRuntimeState>,
    ) -> Result<NodeServiceStatus, String> {
        current_node_service_status(&runtime)
    }


    #[cfg(target_os = "windows")]
    #[tauri::command]
    fn start_node(
        auth_state: tauri::State<'_, Mutex<AppAuthState>>,
        runtime: tauri::State<'_, NodeRuntimeState>,
    ) -> Result<NodeServiceStatus, String> {
        let wallet_password = {
            let state = auth_state
                .lock()
                .map_err(|_| "auth_state_lock_failed".to_string())?;

            if state.authenticated_email.is_none() {
                return Err(
                    "node_start_requires_authenticated_session".into()
                );
            }

            let password = state
                .wallet_password
                .as_ref()
                .ok_or_else(|| {
                    "node_start_wallet_password_missing".to_string()
                })?;

            Zeroizing::new(password.as_str().to_owned())
        };

        crate::core::windows_restart_credential::
            persist_windows_restart_credential_v1(
                wallet_password.as_str(),
            )?;

        crate::core::windows_supervisor_task::
            ensure_windows_supervisor_task_v1()?;

        persist_desired_node_state_v1(
            DesiredNodeStateV1::Running,
            "user_start",
        )?;

        current_node_service_status(&runtime)
    }


    #[cfg(target_os = "macos")]
    #[tauri::command]
    fn start_node(
        auth_state: tauri::State<'_, Mutex<AppAuthState>>,
        runtime: tauri::State<'_, NodeRuntimeState>,
    ) -> Result<NodeServiceStatus, String> {
        let wallet_password = {
            let state = auth_state
                .lock()
                .map_err(|_| {
                    "auth_state_lock_failed".to_string()
                })?;

            if state.authenticated_email.is_none() {
                return Err(
                    "node_start_requires_authenticated_session"
                        .into()
                );
            }

            let password =
                state.wallet_password
                    .as_ref()
                    .ok_or_else(|| {
                        "node_start_wallet_password_missing"
                            .to_string()
                    })?;

            Zeroizing::new(
                password.as_str().to_owned()
            )
        };

        crate::core::macos_supervisor_agent::
            persist_restart_credential_v2(
                wallet_password.as_str()
            )?;

        crate::core::macos_supervisor_agent::
            ensure_launch_agent_v1()?;

        crate::core::macos_supervisor_agent::
            ensure_update_agent_v1()?;

        persist_desired_node_state_v1(
            DesiredNodeStateV1::Running,
            "user_start",
        )?;

        current_node_service_status(&runtime)
    }


    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos"
    )))]
    #[tauri::command]
    fn start_node(
        auth_state: tauri::State<'_, Mutex<AppAuthState>>,
        runtime: tauri::State<'_, NodeRuntimeState>,
    ) -> Result<NodeServiceStatus, String> {
        if runtime.running.load(Ordering::Acquire) {
            return current_node_service_status(&runtime);
        }

        // Reap a previously completed worker before starting another.
        {
            let mut worker = runtime
                .worker
                .lock()
                .map_err(|_| "node_worker_lock_failed".to_string())?;

            if worker
                .as_ref()
                .map(|handle| handle.is_finished())
                .unwrap_or(false)
            {
                if let Some(handle) = worker.take() {
                    let _ = handle.join();
                }
            }
        }

        let wallet_password = {
            let state = auth_state
                .lock()
                .map_err(|_| "auth_state_lock_failed".to_string())?;

            if state.authenticated_email.is_none() {
                return Err("node_start_requires_authenticated_session".into());
            }

            let password = state
                .wallet_password
                .as_ref()
                .ok_or_else(|| "node_start_wallet_password_missing".to_string())?;

            Zeroizing::new(password.as_str().to_owned())
        };

        #[cfg(target_os = "windows")]
        crate::core::windows_restart_credential::
            persist_windows_restart_credential_v1(
                wallet_password.as_str(),
            )?;

        persist_desired_node_state_v1(
            DesiredNodeStateV1::Running,
            "user_start",
        )?;

        clear_node_service_logs();
        runtime.stop.store(false, Ordering::Release);

        {
            let mut error = runtime
                .last_error
                .lock()
                .map_err(|_| "node_service_error_lock_failed".to_string())?;

            *error = None;
        }

        runtime.running.store(true, Ordering::Release);

        let stop = Arc::clone(&runtime.stop);
        let running = Arc::clone(&runtime.running);
        let last_error = Arc::clone(&runtime.last_error);

        let handle = thread::spawn(move || {
            let _power_guard = match crate::core::power_guard::PowerGuard::acquire() {
                Ok(guard) => guard,
                Err(error) => {
                    if let Ok(mut slot) = last_error.lock() {
                        *slot = Some(error);
                    }

                    running.store(false, Ordering::Release);
                    return;
                }
            };

            let result = run_node_service(Arc::clone(&stop), wallet_password);

            if let Err(error) = result {
                if let Ok(mut slot) = last_error.lock() {
                    *slot = Some(error);
                }
            }

            running.store(false, Ordering::Release);
        });

        {
            let mut worker = runtime
                .worker
                .lock()
                .map_err(|_| "node_worker_lock_failed".to_string())?;

            *worker = Some(handle);
        }

        current_node_service_status(&runtime)
    }


    #[cfg(target_os = "windows")]
    #[tauri::command]
    fn stop_node(
        runtime: tauri::State<'_, NodeRuntimeState>,
    ) -> Result<NodeServiceStatus, String> {
        persist_desired_node_state_v1(
            DesiredNodeStateV1::UserStopped,
            "user_stop",
        )?;

        current_node_service_status(&runtime)
    }


    #[cfg(target_os = "macos")]
    #[tauri::command]
    fn stop_node(
        runtime: tauri::State<'_, NodeRuntimeState>,
    ) -> Result<NodeServiceStatus, String> {
        persist_desired_node_state_v1(
            DesiredNodeStateV1::UserStopped,
            "user_stop",
        )?;

        current_node_service_status(&runtime)
    }


    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos"
    )))]
    #[tauri::command]
    fn stop_node(runtime: tauri::State<'_, NodeRuntimeState>) -> Result<NodeServiceStatus, String> {
        persist_desired_node_state_v1(
            DesiredNodeStateV1::UserStopped,
            "user_stop",
        )?;
        if runtime.running.load(Ordering::Acquire) {
            runtime.stop.store(true, Ordering::Release);
        }

        current_node_service_status(&runtime)
    }

    #[cfg(target_os = "windows")]
    fn update_task_control_v1(
        mode: &str,
    ) -> Result<(), String> {
        use std::os::windows::process::CommandExt;

        let exe =
            std::env::current_exe()
                .map_err(|_| "update_current_exe_failed".to_string())?;

        let root =
            exe.parent()
                .ok_or_else(|| "update_install_dir_missing".to_string())?;

        let supervisor =
            root.join("edgeswarm-node-supervisor.exe");

        let script =
            root.join("resources")
                .join("windows")
                .join("supervisor-task.ps1");

        if !script.is_file() {
            return Err("update_task_script_missing".into());
        }

        let status =
            std::process::Command::new("powershell.exe")
                .creation_flags(0x08000000)
                .arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(script)
                .arg("-Mode")
                .arg(mode)
                .arg("-SupervisorPath")
                .arg(supervisor)
                .arg("-AppPath")
                .arg(exe)
                .status()
                .map_err(|_| "update_task_control_launch_failed".to_string())?;

        if !status.success() {
            return Err(format!(
                "update_task_control_failed:{}",
                status.code().unwrap_or(-1)
            ));
        }

        Ok(())
    }

    #[cfg(target_os = "windows")]
    async fn windows_background_update_check_v1(
        app: tauri::AppHandle,
        endpoint_override: Option<String>,
    ) -> Result<(), String> {
        use tauri_plugin_updater::UpdaterExt;

        println!("BACKGROUND_UPDATE_CHECK=true");

        let mut updater_builder =
            app.updater_builder()
                .restart_after_install(false);

        if let Some(endpoint_text) =
            endpoint_override
        {
            let endpoint =
                endpoint_text
                    .parse()
                    .map_err(|e| {
                        format!(
                            "background_update_endpoint_invalid:{e}"
                        )
                    })?;

            updater_builder =
                updater_builder
                    .endpoints(vec![endpoint])
                    .map_err(|e| {
                        format!(
                            "background_update_endpoint_failed:{e}"
                        )
                    })?;

            println!(
                "BACKGROUND_UPDATE_ENDPOINT_OVERRIDE=true"
            );
        }

        let updater =
            updater_builder
                .build()
                .map_err(|e| format!("background_updater_build_failed:{e}"))?;

        let Some(update) =
            updater
                .check()
                .await
                .map_err(|e| format!("background_update_check_failed:{e}"))?
        else {
            println!("BACKGROUND_UPDATE_AVAILABLE=false");
            return Ok(());
        };

        println!("BACKGROUND_UPDATE_AVAILABLE=true");
        println!("BACKGROUND_UPDATE_VERSION={}", update.version);

        let bytes =
            update
                .download(
                    |chunk, total| {
                        println!(
                            "BACKGROUND_UPDATE_DOWNLOAD={chunk}|TOTAL={total:?}"
                        );
                    },
                    || {
                        println!(
                            "BACKGROUND_UPDATE_DOWNLOAD_COMPLETE=true"
                        );
                    },
                )
                .await
                .map_err(|e| {
                    format!("background_update_download_failed:{e}")
                })?;

        // Signature verification is completed by Tauri before
        // download() returns successfully.
        println!("BACKGROUND_UPDATE_SIGNATURE_VERIFIED=true");

        // Only stop paid-work processes after the complete signed
        // updater package is safely available locally.
        update_task_control_v1("PauseForUpdate")?;

        println!("BACKGROUND_UPDATE_PROVIDER_PAUSED=true");

        let update =
            update.restart_after_install(false);

        match update.install(bytes) {
            Ok(()) => {
                println!(
                    "BACKGROUND_UPDATE_INSTALL_DISPATCHED=true"
                );

                Ok(())
            }

            Err(error) => {
                // Installation did not launch successfully.
                // Restore the existing background provider.
                let _ =
                    update_task_control_v1("Install");

                Err(format!(
                    "background_update_install_failed:{error}"
                ))
            }
        }
    }


    #[cfg(target_os = "macos")]
    async fn macos_background_update_check_v1(
        app: tauri::AppHandle,
        endpoint_override: Option<String>,
    ) -> Result<(), String> {
        use tauri_plugin_updater::UpdaterExt;

        println!(
            "MACOS_BACKGROUND_UPDATE_CHECK=true"
        );

        let mut updater_builder =
            app.updater_builder()
                .restart_after_install(false);

        if let Some(endpoint_text) =
            endpoint_override
        {
            let endpoint =
                endpoint_text
                    .parse()
                    .map_err(|e| {
                        format!(
                            "macos_background_update_endpoint_invalid:{e}"
                        )
                    })?;

            updater_builder =
                updater_builder
                    .endpoints(vec![endpoint])
                    .map_err(|e| {
                        format!(
                            "macos_background_update_endpoint_failed:{e}"
                        )
                    })?;

            println!(
                "MACOS_BACKGROUND_UPDATE_ENDPOINT_OVERRIDE=true"
            );
        }

        let updater =
            updater_builder
                .build()
                .map_err(|e| {
                    format!(
                        "macos_background_updater_build_failed:{e}"
                    )
                })?;

        let Some(update) =
            updater
                .check()
                .await
                .map_err(|e| {
                    format!(
                        "macos_background_update_check_failed:{e}"
                    )
                })?
        else {
            println!(
                "MACOS_BACKGROUND_UPDATE_AVAILABLE=false"
            );

            return Ok(());
        };

        println!(
            "MACOS_BACKGROUND_UPDATE_AVAILABLE=true"
        );

        println!(
            "MACOS_BACKGROUND_UPDATE_VERSION={}",
            update.version
        );

        // Download and signature-verify the entire updater package
        // BEFORE interrupting the provider.
        let bytes =
            update
                .download(
                    |chunk, total| {
                        println!(
                            "MACOS_BACKGROUND_UPDATE_DOWNLOAD={chunk}|TOTAL={total:?}"
                        );
                    },
                    || {
                        println!(
                            "MACOS_BACKGROUND_UPDATE_DOWNLOAD_COMPLETE=true"
                        );
                    },
                )
                .await
                .map_err(|e| {
                    format!(
                        "macos_background_update_download_failed:{e}"
                    )
                })?;

        println!(
            "MACOS_BACKGROUND_UPDATE_SIGNATURE_VERIFIED=true"
        );

        // Ensure the persistent supervisor exists before placing
        // the provider into update pause.
        crate::core::macos_supervisor_agent::
            ensure_launch_agent_v1()?;

        crate::core::macos_supervisor_agent::
            begin_update_pause_v1()?;

        // Restarting the supervisor immediately makes it observe
        // the pause. The old headless process also has a parent
        // guard and exits if its old supervisor disappears.
        let prepare_result =
            (|| -> Result<(), String> {
                crate::core::macos_supervisor_agent::
                    kickstart_supervisor_v1()?;

                crate::core::macos_supervisor_agent::
                    wait_for_provider_paused_v1(
                        std::time::Duration::from_secs(20)
                    )?;

                Ok(())
            })();

        if let Err(error) =
            prepare_result
        {
            let _ =
                crate::core::macos_supervisor_agent::
                    clear_update_pause_v1();

            let _ =
                crate::core::macos_supervisor_agent::
                    kickstart_supervisor_v1();

            return Err(error);
        }

        println!(
            "MACOS_BACKGROUND_UPDATE_PROVIDER_PAUSED=true"
        );

        let update =
            update.restart_after_install(false);

        let install_result =
            update
                .install(bytes)
                .map_err(|e| {
                    format!(
                        "macos_background_update_install_failed:{e}"
                    )
                });

        match install_result {
            Ok(()) => {
                crate::core::macos_supervisor_agent::
                    clear_update_pause_v1()?;

                crate::core::macos_supervisor_agent::
                    kickstart_supervisor_v1()?;

                println!(
                    "MACOS_BACKGROUND_UPDATE_INSTALL_COMPLETE=true"
                );

                println!(
                    "MACOS_BACKGROUND_UPDATE_PROVIDER_RECOVERED=true"
                );

                Ok(())
            }

            Err(error) => {
                // Fail open for provider availability after a failed
                // update attempt while preserving desired_state.json.
                let _ =
                    crate::core::macos_supervisor_agent::
                        clear_update_pause_v1();

                let _ =
                    crate::core::macos_supervisor_agent::
                        kickstart_supervisor_v1();

                println!(
                    "MACOS_BACKGROUND_UPDATE_PROVIDER_RECOVERY_ATTEMPTED=true"
                );

                Err(error)
            }
        }
    }


    #[cfg(target_os = "linux")]
    async fn desktop_update_check_v1(
        app: tauri::AppHandle,
    ) -> tauri_plugin_updater::Result<()> {
        use tauri_plugin_updater::UpdaterExt;

        let Some(update) = app.updater()?.check().await? else {
            println!("DESKTOP_UPDATE_AVAILABLE=false");
            return Ok(());
        };

        let version = update.version.clone();
        println!("DESKTOP_UPDATE_AVAILABLE=true");
        println!("DESKTOP_UPDATE_VERSION={version}");

        let mut downloaded = 0usize;
        update
            .download_and_install(
                |chunk_length, content_length| {
                    downloaded += chunk_length;
                    println!(
                        "DESKTOP_UPDATE_PROGRESS_BYTES={downloaded}|TOTAL={content_length:?}"
                    );
                },
                || println!("DESKTOP_UPDATE_DOWNLOAD_COMPLETE=true"),
            )
            .await?;

        println!("DESKTOP_UPDATE_INSTALLED={version}");
        app.restart();
    }
    #[cfg_attr(mobile, tauri::mobile_entry_point)]
    pub fn run() {
        tauri::Builder::default()
            .manage(Mutex::new(AppAuthState::default()))
            .manage(NodeRuntimeState::default())
            .plugin(tauri_plugin_opener::init())
            .plugin(tauri_plugin_updater::Builder::new().build())
            .setup(|app| {
                #[cfg(target_os = "windows")]
                {
                    let args =
                        std::env::args()
                            .collect::<Vec<_>>();

                    let background_update =
                        args.iter()
                            .any(|arg| {
                                arg == "--background-update-check"
                            });

                    let background_update_endpoint =
                        args.windows(2)
                            .find_map(|pair| {
                                if pair[0] ==
                                    "--background-update-endpoint"
                                {
                                    Some(pair[1].clone())
                                } else {
                                    None
                                }
                            });

                    if background_update {
                        let handle =
                            app.handle().clone();

                        tauri::async_runtime::spawn(
                            async move {
                                // BACKGROUND_UPDATE_EXIT_STATUS_V1
                                let exit_code =
                                    match windows_background_update_check_v1(
                                        handle.clone(),
                                        background_update_endpoint,
                                    ).await
                                    {
                                        Ok(()) => {
                                            println!(
                                                "BACKGROUND_UPDATE_RESULT=success"
                                            );

                                            0
                                        }

                                        Err(error) => {
                                            eprintln!(
                                                "BACKGROUND_UPDATE_ERROR={error}"
                                            );

                                            1
                                        }
                                    };

                                handle.exit(exit_code);
                            }
                        );

                        return Ok(());
                    }

                    if let Some(window) =
                        app.get_webview_window("main")
                    {
                        window.show()?;
                    }
                }

                #[cfg(target_os = "macos")]
                {
                    let args =
                        std::env::args()
                            .collect::<Vec<_>>();

                    let background_update =
                        args.iter()
                            .any(|arg| {
                                arg ==
                                    "--background-update-check"
                            });

                    let background_update_endpoint =
                        args.windows(2)
                            .find_map(|pair| {
                                if pair[0] ==
                                    "--background-update-endpoint"
                                {
                                    Some(pair[1].clone())
                                } else {
                                    None
                                }
                            });

                    if background_update {
                        let handle =
                            app.handle().clone();

                        tauri::async_runtime::spawn(
                            async move {
                                let exit_code =
                                    match
                                        macos_background_update_check_v1(
                                            handle.clone(),
                                            background_update_endpoint,
                                        ).await
                                    {
                                        Ok(()) => {
                                            println!(
                                                "MACOS_BACKGROUND_UPDATE_RESULT=success"
                                            );

                                            0
                                        }

                                        Err(error) => {
                                            eprintln!(
                                                "MACOS_BACKGROUND_UPDATE_ERROR={error}"
                                            );

                                            1
                                        }
                                    };

                                handle.exit(
                                    exit_code
                                );
                            }
                        );

                        return Ok(());
                    }

                    match
                        crate::core::macos_supervisor_agent::
                            ensure_update_agent_v1()
                    {
                        Ok(()) => {
                            println!(
                                "MACOS_UPDATE_AGENT_READY=true"
                            );
                        }

                        Err(error) => {
                            println!(
                                "MACOS_UPDATE_AGENT_ERROR={error}"
                            );
                        }
                    }
                }

                #[cfg(not(target_os = "windows"))]
                {
                    if let Some(window) =
                        app.get_webview_window("main")
                    {
                        window.show()?;

                        #[cfg(target_os = "macos")]
                        window.set_focus()?;
                    }

                    #[cfg(target_os = "linux")]
                    {
                        let handle =
                            app.handle().clone();

                        std::thread::spawn(move || loop {
                            let check_handle =
                                handle.clone();

                            tauri::async_runtime::block_on(
                                async move {
                                    if let Err(error) =
                                        desktop_update_check_v1(
                                            check_handle
                                        ).await
                                    {
                                        println!(
                                            "DESKTOP_UPDATE_ERROR={error}"
                                        );
                                    }
                                }
                            );

                            std::thread::sleep(
                                std::time::Duration::from_secs(
                                    60 * 60
                                )
                            );
                        });
                    }
                }

                Ok(())
            })
            .invoke_handler(tauri::generate_handler![
                get_node_state,
                set_window_layout,
                auth_begin,
                auth_verify,
                provider_ledger_sync,
                node_service_status,
                request_capacity_test,
                start_node,
                stop_node
            ])
            .run(tauri::generate_context!())
            .expect("error while running EdgeSwarm Unified Node");
    }
}

#[cfg(feature = "desktop")]
pub use desktop::run;
