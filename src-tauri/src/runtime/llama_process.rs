use crate::adapters;
use reqwest::blocking::Client;
use serde_json::Value;
use std::{
    env,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const LLAMA_DEFAULT_HOST: &str = "127.0.0.1";
pub const LLAMA_DEFAULT_PORT: u16 = 18081;
pub const LLAMA_MODEL_ALIAS: &str = "local-model";

#[derive(Debug, Clone)]
pub struct LlamaProcessConfig {
    pub executable: PathBuf,
    pub model_path: PathBuf,
    pub host: String,
    pub port: u16,
    pub context_tokens: u32,
    pub threads: u32,
    pub gpu_layers: i32,
    pub startup_timeout: Duration,
}

fn runtime_has_gpu_backend_v1(executable: &std::path::Path) -> bool {
    executable
        .parent()
        .map(|directory| {
            directory.join("ggml-cuda.dll").is_file()
                || directory.join("ggml-vulkan.dll").is_file()
        })
        .unwrap_or(false)
}
pub fn runtime_acceleration_for_config_v1(
    config: &LlamaProcessConfig,
) -> String {
    if config.gpu_layers == 0 {
        return "cpu".into();
    }

    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return "metal".into();
    }

    if let Some(directory) = config.executable.parent() {
        if let Ok(entries) = std::fs::read_dir(directory) {
            let names = entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .map(|name| name.to_ascii_lowercase())
                .collect::<Vec<_>>();

            if names.iter().any(|name| name.contains("ggml-cuda")) {
                return "cuda".into();
            }

            if names.iter().any(|name| name.contains("ggml-vulkan")) {
                return "vulkan".into();
            }
        }
    }

    let detected = adapters::detect_acceleration().backend;

    if matches!(detected.as_str(), "cuda" | "vulkan" | "metal") {
        return detected;
    }

    "cpu".into()
}
impl LlamaProcessConfig {
    pub fn for_model(model_path: impl Into<PathBuf>) -> Result<Self, String> {
        let executable = resolve_llama_server_path_v1()?;

        let gpu_layers = env::var("EDGESWARM_LLAMA_GPU_LAYERS")
            .ok()
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or_else(|| {
                if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
                    999
                } else if cfg!(target_os = "windows")
                    && executable
                        .parent()
                        .map(|directory| {
                            directory.join("ggml-cuda.dll").is_file()
                                || directory.join("ggml-vulkan.dll").is_file()
                        })
                        .unwrap_or(false)
                {
                    -1
                } else {
                    0
                }
            });

        Ok(Self {
            executable,
            model_path: model_path.into(),
            host: LLAMA_DEFAULT_HOST.into(),
            port: LLAMA_DEFAULT_PORT,
            context_tokens: 4096,
            threads: 8,
            gpu_layers,
            startup_timeout: Duration::from_secs(180),
        })
    }
    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

pub struct ManagedLlamaProcess {
    child: Child,
    base_url: String,
    executable: PathBuf,
    acceleration: String,
}

impl ManagedLlamaProcess {
    pub fn start(config: &LlamaProcessConfig) -> Result<Self, String> {
        validate_config(config)?;

        let mut command = Command::new(&config.executable);

        command
            .arg("-m")
            .arg(&config.model_path)
            .arg("--alias")
            .arg(LLAMA_MODEL_ALIAS)
            .arg("--host")
            .arg(&config.host)
            .arg("--port")
            .arg(config.port.to_string())
            .arg("--ctx-size")
            .arg(config.context_tokens.to_string())
            .arg("--threads")
            .arg(config.threads.to_string())
            .arg("--n-gpu-layers")
            .arg(config.gpu_layers.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let child = command
            .spawn()
            .map_err(|e| format!("llama_server_spawn_failed:{e}"))?;

        let mut managed = Self {
            child,
            base_url: config.base_url(),
            executable: config.executable.clone(),
            acceleration: runtime_acceleration_for_config_v1(config),
        };

        if let Err(error) = managed.wait_until_ready(config.startup_timeout) {
            managed.stop();
            return Err(error);
        }

        Ok(managed)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
    pub fn executable_path(&self) -> &std::path::Path {
        &self.executable
    }

    pub fn acceleration(&self) -> &str {
        &self.acceleration
    }

    pub fn stop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    fn wait_until_ready(&mut self, timeout: Duration) -> Result<(), String> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3))
            .no_proxy()
            .build()
            .map_err(|e| format!("llama_health_client_failed:{e}"))?;

        let deadline = Instant::now() + timeout;
        let health_url = format!("{}/health", self.base_url);

        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(format!("llama_server_exited_before_ready:{status}"));
            }

            if let Ok(response) = client.get(&health_url).send() {
                if response.status().is_success() {
                    let healthy = response
                        .json::<Value>()
                        .ok()
                        .and_then(|value| {
                            value
                                .get("status")
                                .and_then(Value::as_str)
                                .map(|status| status.eq_ignore_ascii_case("ok"))
                        })
                        .unwrap_or(true);

                    if healthy {
                        return Ok(());
                    }
                }
            }

            thread::sleep(Duration::from_millis(500));
        }

        Err("llama_server_startup_timeout".into())
    }
}

impl Drop for ManagedLlamaProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn resolve_cpu_llama_server_path_v1() -> Result<PathBuf, String> {
    let filename = runtime_executable_filename_v1();

    if let Ok(executable) = env::current_exe() {
        if let Some(dir) = executable.parent() {
            for path in [
                dir.join("resources").join("runtime").join("current").join(filename),
                dir.join("runtime").join("current").join(filename),
                dir.join("runtime").join(filename),
            ] {
                if path.is_file() {
                    return Ok(path);
                }
            }
        }
    }

    Err("cpu_llama_server_not_installed".into())
}

pub fn start_managed_llama_with_cpu_fallback_v1(
    config: &LlamaProcessConfig,
) -> Result<ManagedLlamaProcess, String> {
    let requested = runtime_acceleration_for_config_v1(config);

    match ManagedLlamaProcess::start(config) {
        Ok(runtime) => Ok(runtime),
        Err(primary_error) if requested == "cuda" || requested == "vulkan" => {
            println!("LLAMA_ACCELERATED_RUNTIME_FAILED={primary_error}");
            println!("LLAMA_CPU_FALLBACK_ATTEMPTED=true");

            let mut fallback = config.clone();
            fallback.executable = resolve_cpu_llama_server_path_v1()?;
            fallback.gpu_layers = 0;

            let runtime = ManagedLlamaProcess::start(&fallback)?;
            println!("LLAMA_CPU_FALLBACK_ACTIVE=true");
            Ok(runtime)
        }
        Err(error) => Err(error),
    }
}
pub fn resolve_model_root_v1() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("EDGESWARM_MODEL_ROOT") {
        let path = PathBuf::from(path);

        if path.is_dir() {
            return Ok(path);
        }

        return Err("configured_model_root_not_directory".into());
    }

    let app_data = adapters::app_data_dir();
    let mut candidates = Vec::new();

    if let Some(parent) = app_data.parent() {
        candidates.push(parent.join("models"));
    }

    candidates.push(app_data.join("models"));

    candidates
        .into_iter()
        .find(|path| path.is_dir())
        .ok_or_else(|| "model_root_not_installed".into())
}

fn find_runtime_executable_v1(
    root: &std::path::Path,
    filename: &str,
    depth: u8,
) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }

    let mut entries = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();

    entries.sort();

    for path in &entries {
        if path.is_file()
            && path.file_name().and_then(|value| value.to_str()) == Some(filename)
        {
            return Some(path.clone());
        }
    }

    for path in entries {
        if path.is_dir() {
            if let Some(found) =
                find_runtime_executable_v1(&path, filename, depth - 1)
            {
                return Some(found);
            }
        }
    }

    None
}

pub fn resolve_llama_server_path_v1() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("EDGESWARM_LLAMA_SERVER_PATH") {
        let path = PathBuf::from(path);

        if path.is_file() {
            return Ok(path);
        }

        return Err("configured_llama_server_not_file".into());
    }

    let filename = runtime_executable_filename_v1();

    // WINDOWS_ACCELERATED_RUNTIME_SELECTION_V1
    #[cfg(target_os = "windows")]
    {
        let acceleration = adapters::detect_acceleration();

        let variant = match acceleration.backend.as_str() {
            "cuda" => Some("cuda"),
            "vulkan" => Some("vulkan"),
            _ => None,
        };

        if let Some(variant) = variant {
            if let Ok(executable) = env::current_exe() {
                if let Some(directory) = executable.parent() {
                    let path = directory
                        .join("resources")
                        .join("runtime")
                        .join(variant)
                        .join("current")
                        .join(filename);

                    if path.is_file() {
                        println!("LLAMA_RUNTIME_VARIANT={variant}");
                        return Ok(path);
                    }
                }
            }

            println!(
                "LLAMA_ACCELERATED_RUNTIME_UNAVAILABLE={}",
                acceleration.backend
            );
        }
    }
    // UNIFIED_BUNDLED_LLAMA_RUNTIME_V1
    // Each supported release ships its native llama runtime under
    // runtime/current beside the EdgeSwarm executable.
    if let Ok(executable) = env::current_exe() {
        if let Some(executable_dir) = executable.parent() {
            let packaged = [
                executable_dir
                    .join("runtime")
                    .join("current")
                    .join(filename),
                executable_dir
                    .join("runtime")
                    .join(filename),
                executable_dir
                    .join("resources")
                    .join("runtime")
                    .join("current")
                    .join(filename),
            ];

            if let Some(path) =
                packaged.into_iter().find(|path| path.is_file())
            {
                return Ok(path);
            }
        }
    }

    // Preserve the existing locally provisioned runtime fallback.
    let runtime_root = adapters::app_data_dir().join("runtime");

    let candidates = [
        runtime_root.join("current").join(filename),
        runtime_root.join(filename),
    ];

    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Ok(path);
    }

    find_runtime_executable_v1(&runtime_root, filename, 4)
        .ok_or_else(|| "llama_server_not_installed".into())
}

fn runtime_executable_filename_v1() -> &'static str {
    if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

fn validate_config(config: &LlamaProcessConfig) -> Result<(), String> {
    if !config.executable.is_file() {
        return Err("llama_server_not_file".into());
    }

    if !config.model_path.is_file() {
        return Err("llama_model_not_file".into());
    }

    if config.host != "127.0.0.1" && config.host != "localhost" {
        return Err("llama_server_non_localhost_rejected".into());
    }

    if config.port == 0 {
        return Err("llama_server_invalid_port".into());
    }

    if config.context_tokens == 0 || config.threads == 0 {
        return Err("llama_server_invalid_runtime_config".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_local() {
        let config = LlamaProcessConfig {
            executable: PathBuf::from("server"),
            model_path: PathBuf::from("model.gguf"),
            host: "127.0.0.1".into(),
            port: 18081,
            context_tokens: 4096,
            threads: 8,
            gpu_layers: 0,
            startup_timeout: Duration::from_secs(1),
        };

        assert_eq!(config.base_url(), "http://127.0.0.1:18081");
    }

    #[test]
    fn model_alias_matches_http_executor_contract() {
        assert_eq!(LLAMA_MODEL_ALIAS, "local-model");
    }

    #[test]
    fn executable_name_matches_platform() {
        if cfg!(target_os = "windows") {
            assert_eq!(runtime_executable_filename_v1(), "llama-server.exe");
        } else {
            assert_eq!(runtime_executable_filename_v1(), "llama-server");
        }
    }
}
