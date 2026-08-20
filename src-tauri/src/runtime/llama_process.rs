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

impl LlamaProcessConfig {
    pub fn for_model(model_path: impl Into<PathBuf>) -> Result<Self, String> {
        Ok(Self {
            executable: resolve_llama_server_path_v1()?,
            model_path: model_path.into(),
            host: LLAMA_DEFAULT_HOST.into(),
            port: LLAMA_DEFAULT_PORT,
            context_tokens: 4096,
            threads: 8,
            gpu_layers: std::env::var("EDGESWARM_LLAMA_GPU_LAYERS")
                .ok()
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or_else(|| {
                    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
                        999
                    } else {
                        0
                    }
                }),
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

pub fn resolve_llama_server_path_v1() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("EDGESWARM_LLAMA_SERVER_PATH") {
        let path = PathBuf::from(path);

        if path.is_file() {
            return Ok(path);
        }

        return Err("configured_llama_server_not_file".into());
    }

    let filename = runtime_executable_filename_v1();
    let candidates = [
        adapters::app_data_dir()
            .join("runtime")
            .join("current")
            .join(filename),
        adapters::app_data_dir().join("runtime").join(filename),
    ];

    candidates
        .into_iter()
        .find(|path| path.is_file())
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
