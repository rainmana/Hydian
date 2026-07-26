use std::{
    fs::{self, File},
    path::PathBuf,
    process::Stdio,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use tokio::{process::Command, time::Duration};

use crate::{
    config::HydianConfig,
    paths::HydianPaths,
    secrets::{redact_json, redact_text},
    service::display_command,
};

#[derive(Debug, Clone, Serialize)]
pub struct ProviderDetection {
    pub provider: String,
    pub executable: Option<PathBuf>,
    pub version: Option<String>,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExposurePlan {
    pub provider: String,
    pub detection: ProviderDetection,
    pub local_url: String,
    #[serde(skip_serializing)]
    pub command: Vec<String>,
    pub command_display: String,
    pub expected_scope: String,
    pub authentication: String,
    pub tls: String,
    pub limitations: Vec<String>,
    pub experimental: bool,
    pub long_running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposureState {
    pub schema_version: u32,
    pub provider: String,
    pub started_at: DateTime<Utc>,
    pub pid: Option<u32>,
    #[serde(default)]
    pub running: bool,
    pub local_url: String,
    pub public_url: Option<String>,
    pub scope: String,
    pub command: Vec<String>,
    pub log_file: Option<PathBuf>,
}

#[async_trait]
pub trait ExposureProvider: Send + Sync {
    async fn detect(&self) -> ProviderDetection;
    fn validate(&self, scope: Option<&str>, mode: Option<&str>, arguments: &[String])
    -> Result<()>;
    async fn plan(
        &self,
        config: &HydianConfig,
        scope: Option<&str>,
        mode: Option<&str>,
        arguments: &[String],
    ) -> Result<ExposurePlan>;
    async fn start(&self, plan: &ExposurePlan, paths: &HydianPaths) -> Result<ExposureState>;
    async fn status(&self, paths: &HydianPaths) -> Result<Option<ExposureState>>;
    async fn stop(&self, paths: &HydianPaths) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct CommandProvider {
    name: String,
}

impl CommandProvider {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_ascii_lowercase(),
        }
    }

    fn executable_name(&self) -> Option<&'static str> {
        match self.name.as_str() {
            "tailscale" => Some("tailscale"),
            "ngrok" => Some("ngrok"),
            "cloudflare" => Some("cloudflared"),
            _ => None,
        }
    }
}

#[async_trait]
impl ExposureProvider for CommandProvider {
    async fn detect(&self) -> ProviderDetection {
        let executable = if self.name == "custom" {
            None
        } else {
            self.executable_name()
                .and_then(|name| which::which(name).ok())
        };
        let version = if let Some(path) = &executable {
            Command::new(path)
                .arg("--version")
                .stdin(Stdio::null())
                .output()
                .await
                .ok()
                .filter(|output| output.status.success())
                .map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_owned()
                })
        } else {
            None
        };
        ProviderDetection {
            provider: self.name.clone(),
            available: self.name == "custom" || executable.is_some(),
            executable,
            version,
        }
    }

    fn validate(
        &self,
        scope: Option<&str>,
        mode: Option<&str>,
        arguments: &[String],
    ) -> Result<()> {
        match self.name.as_str() {
            "tailscale" if !matches!(scope.unwrap_or("tailnet"), "tailnet" | "public") => {
                bail!("Tailscale scope must be `tailnet` or `public`")
            }
            "cloudflare" if !matches!(mode.unwrap_or("quick"), "quick" | "existing") => {
                bail!("Cloudflare mode must be `quick` or `existing`")
            }
            "cloudflare" if mode == Some("existing") && arguments.is_empty() => {
                bail!("Cloudflare existing mode requires provider-native arguments after `--`")
            }
            "custom" if arguments.is_empty() => {
                bail!("custom provider requires a command after `--`")
            }
            "ngrok"
                if arguments.iter().any(|argument| {
                    argument.eq_ignore_ascii_case("--authtoken")
                        || argument.to_ascii_lowercase().starts_with("--authtoken=")
                }) =>
            {
                bail!(
                    "do not pass an ngrok authentication token on Hydian's command line; configure the ngrok agent credential store first"
                )
            }
            "tailscale" | "ngrok" | "cloudflare" | "custom" => {}
            _ => bail!(
                "unknown exposure provider `{}`; choose tailscale, ngrok, cloudflare, or custom",
                self.name
            ),
        }
        Ok(())
    }

    async fn plan(
        &self,
        config: &HydianConfig,
        scope: Option<&str>,
        mode: Option<&str>,
        arguments: &[String],
    ) -> Result<ExposurePlan> {
        self.validate(scope, mode, arguments)?;
        let detection = self.detect().await;
        if !detection.available {
            bail!(
                "{} executable was not found on PATH; install it from the provider and rerun the plan",
                self.executable_name().unwrap_or(&self.name)
            );
        }
        let local_url = config.endpoint();
        let (command, expected_scope, authentication, tls, limitations, experimental, long_running) =
            match self.name.as_str() {
                "tailscale" => {
                    let scope = scope.unwrap_or("tailnet");
                    let mode_command = if scope == "public" { "funnel" } else { "serve" };
                    (
                        vec![
                            detection.executable.as_ref().unwrap().to_string_lossy().into_owned(),
                            mode_command.into(),
                            "--bg".into(),
                            local_url.clone(),
                        ],
                        if scope == "public" {
                            "public internet (Tailscale Funnel)"
                        } else {
                            "tailnet only (Tailscale Serve)"
                        }
                        .into(),
                        if scope == "public" {
                            "Funnel is publicly reachable; Hydian adds no client authentication."
                        } else {
                            "Tailnet access policy and Tailscale identity protect reachability; identity headers may be present."
                        }
                        .into(),
                        "Tailscale terminates HTTPS; Hydian remains loopback HTTP.".into(),
                        vec!["The assigned HTTPS URL is read from Tailscale's JSON status.".into()],
                        false,
                        false,
                    )
                }
                "ngrok" => {
                    let mut command = vec![
                        detection
                            .executable
                            .as_ref()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        "http".into(),
                        local_url.clone(),
                    ];
                    command.extend(arguments.iter().cloned());
                    (
                        command,
                        "public internet".into(),
                        "ngrok account and traffic policy determine authentication; Hydian adds none.".into(),
                        "ngrok terminates public HTTPS; Hydian remains loopback HTTP.".into(),
                        vec!["The assigned URL is obtained from the local ngrok Agent API.".into()],
                        false,
                        true,
                    )
                }
                "cloudflare" => {
                    let selected = mode.unwrap_or("quick");
                    let mut command = vec![
                        detection
                            .executable
                            .as_ref()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                    ];
                    if selected == "quick" {
                        command.extend(["tunnel".into(), "--url".into(), local_url.clone()]);
                    } else {
                        command.extend(arguments.iter().map(|argument| expand(argument, config)));
                    }
                    (
                        command,
                        if selected == "quick" {
                            "public development tunnel"
                        } else {
                            "operator-configured Cloudflare Tunnel"
                        }
                        .into(),
                        "Cloudflare configuration determines authentication; Quick Tunnels are public.".into(),
                        "Cloudflare terminates public HTTPS; Hydian remains HTTP.".into(),
                        if selected == "quick" {
                            vec![
                                "Cloudflare Quick Tunnels are development-only.".into(),
                                "Official Cloudflare documentation says Quick Tunnels do not support SSE; Streamable HTTP compatibility is therefore experimental and not claimed as complete.".into(),
                                "Use a named tunnel for serious use.".into(),
                            ]
                        } else {
                            vec!["Hydian does not recreate Cloudflare's control plane; provider-native arguments are used as supplied.".into()]
                        },
                        selected == "quick",
                        true,
                    )
                }
                "custom" => (
                    arguments
                        .iter()
                        .map(|argument| expand(argument, config))
                        .collect(),
                    "operator-defined".into(),
                    "The custom command determines authentication.".into(),
                    "The custom command determines TLS termination.".into(),
                    vec![
                        "Hydian cannot infer the assigned URL or provider security policy.".into(),
                    ],
                    true,
                    true,
                ),
                _ => unreachable!(),
            };
        let command_display = display_command(&redact_command(&command));
        Ok(ExposurePlan {
            provider: self.name.clone(),
            detection,
            local_url,
            command_display,
            command,
            expected_scope,
            authentication,
            tls,
            limitations,
            experimental,
            long_running,
        })
    }

    async fn start(&self, plan: &ExposurePlan, paths: &HydianPaths) -> Result<ExposureState> {
        let (program, arguments) = plan
            .command
            .split_first()
            .ok_or_else(|| anyhow!("provider command is empty"))?;
        let log_file = paths.logs.join(format!("exposure-{}.log", plan.provider));
        let mut state = ExposureState {
            schema_version: 1,
            provider: plan.provider.clone(),
            started_at: Utc::now(),
            pid: None,
            running: true,
            local_url: plan.local_url.clone(),
            public_url: None,
            scope: plan.expected_scope.clone(),
            command: redact_command(&plan.command),
            log_file: Some(log_file.clone()),
        };
        if plan.long_running {
            fs::create_dir_all(&paths.logs)?;
            let stdout = File::create(&log_file)?;
            let stderr = stdout.try_clone()?;
            let child = Command::new(program)
                .args(arguments)
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr))
                .kill_on_drop(false)
                .spawn()
                .with_context(|| format!("could not start {}", plan.command_display))?;
            state.pid = child.id();
            drop(child);
            if plan.provider == "ngrok" {
                state.public_url = discover_ngrok_url().await;
            }
        } else {
            let output = Command::new(program)
                .args(arguments)
                .stdin(Stdio::null())
                .output()
                .await?;
            if !output.status.success() {
                bail!(
                    "provider command failed: {}",
                    redact_text(&String::from_utf8_lossy(&output.stderr), &[])
                );
            }
            if plan.provider == "tailscale" {
                state.public_url = tailscale_url(program).await;
            }
        }
        write_state(paths, &state)?;
        Ok(state)
    }

    async fn status(&self, paths: &HydianPaths) -> Result<Option<ExposureState>> {
        let Some(mut state) = read_state(paths)? else {
            return Ok(None);
        };
        if let Some(pid) = state.pid {
            state.running = pid_alive(pid).await;
        }
        Ok(Some(state))
    }

    async fn stop(&self, paths: &HydianPaths) -> Result<()> {
        let Some(state) = read_state(paths)? else {
            return Ok(());
        };
        if state.provider == "tailscale" {
            if let Ok(executable) = which::which("tailscale") {
                let mode = if state.scope.contains("public") {
                    "funnel"
                } else {
                    "serve"
                };
                let _ = Command::new(executable)
                    .args([mode, "--https=443", "off"])
                    .output()
                    .await;
            }
        } else if let Some(pid) = state.pid {
            stop_pid(pid).await?;
        }
        let state_path = state_path(paths);
        if state_path.exists() {
            fs::remove_file(state_path)?;
        }
        Ok(())
    }
}

fn expand(argument: &str, config: &HydianConfig) -> String {
    argument
        .replace("{local_url}", &config.endpoint())
        .replace("{local_host}", &config.listener.host)
        .replace("{local_port}", &config.listener.port.to_string())
        .replace("{mcp_path}", &config.listener.path)
}

async fn discover_ngrok_url() -> Option<String> {
    let client = reqwest::Client::new();
    for _ in 0..20 {
        for endpoint in [
            "http://127.0.0.1:4040/api/endpoints",
            "http://127.0.0.1:4040/api/tunnels",
        ] {
            if let Ok(value) = client
                .get(endpoint)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                && let Ok(value) = value.json::<serde_json::Value>().await
            {
                let redacted = redact_json(&value);
                if let Some(url) = redacted["endpoints"]
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item["url"].as_str())
                    .or_else(|| {
                        redacted["tunnels"]
                            .as_array()
                            .and_then(|items| items.first())
                            .and_then(|item| item["public_url"].as_str())
                    })
                {
                    return Some(url.to_owned());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

async fn tailscale_url(program: &str) -> Option<String> {
    let output = Command::new(program)
        .args(["serve", "status", "--json"])
        .output()
        .await
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    find_https_url(&value)
}

fn find_https_url(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if text.starts_with("https://") => Some(text.clone()),
        serde_json::Value::Array(items) => items.iter().find_map(find_https_url),
        serde_json::Value::Object(map) => map.values().find_map(find_https_url),
        _ => None,
    }
}

async fn stop_pid(pid: u32) -> Result<()> {
    let output = if cfg!(windows) {
        Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output()
            .await
    } else {
        Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output()
            .await
    }
    .context("could not invoke process termination command")?;
    if !output.status.success() {
        bail!(
            "could not stop provider process {pid}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

async fn pid_alive(pid: u32) -> bool {
    let output = if cfg!(windows) {
        Command::new("tasklist.exe")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .await
    } else {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .await
    };
    output.is_ok_and(|output| {
        output.status.success()
            && (!cfg!(windows)
                || String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
    })
}

fn redact_command(command: &[String]) -> Vec<String> {
    let mut redact_next = false;
    command
        .iter()
        .map(|argument| {
            if redact_next {
                redact_next = false;
                return "[REDACTED]".into();
            }
            if let Some((key, _)) = argument.split_once('=')
                && crate::secrets::is_secret_key(key)
            {
                return format!("{key}=[REDACTED]");
            }
            if argument.starts_with('-') && crate::secrets::is_secret_key(argument) {
                redact_next = true;
            }
            argument.clone()
        })
        .collect()
}

fn state_path(paths: &HydianPaths) -> PathBuf {
    paths.run.join("exposure.json")
}

fn write_state(paths: &HydianPaths, state: &ExposureState) -> Result<()> {
    fs::create_dir_all(&paths.run)?;
    let path = state_path(paths);
    let bytes = serde_json::to_vec_pretty(state)?;
    let mut temporary = NamedTempFile::new_in(&paths.run)?;
    std::io::Write::write_all(&mut temporary, &bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("could not replace exposure status {}", path.display()))?;
    Ok(())
}

fn read_state(paths: &HydianPaths) -> Result<Option<ExposureState>> {
    let path = state_path(paths);
    if !path.exists() {
        return Ok(None);
    }
    let input = fs::read_to_string(&path)?;
    serde_json::from_str(&input)
        .with_context(|| format!("exposure status is invalid at {}", path.display()))
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::{CommandProvider, ExposureProvider};
    use crate::config::HydianConfig;

    #[tokio::test]
    async fn custom_provider_expands_every_placeholder() {
        let provider = CommandProvider::new("custom");
        let plan = provider
            .plan(
                &HydianConfig::default(),
                None,
                None,
                &[
                    "my-tunnel".into(),
                    "--upstream".into(),
                    "{local_url}".into(),
                    "--host".into(),
                    "{local_host}".into(),
                    "--port".into(),
                    "{local_port}".into(),
                    "--path".into(),
                    "{mcp_path}".into(),
                ],
            )
            .await
            .unwrap();
        assert!(plan.command.contains(&"http://127.0.0.1:7337/mcp".into()));
        assert!(plan.command.contains(&"127.0.0.1".into()));
        assert!(plan.command.contains(&"7337".into()));
        assert!(plan.command.contains(&"/mcp".into()));
    }

    #[tokio::test]
    async fn missing_provider_executable_has_a_concrete_diagnostic() {
        let provider = CommandProvider::new("unknown");
        let error = provider
            .plan(&HydianConfig::default(), None, None, &[])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unknown exposure provider"));
    }

    #[test]
    fn ngrok_tokens_are_rejected_from_process_arguments() {
        let provider = CommandProvider::new("ngrok");
        let error = provider
            .validate(None, None, &["--authtoken=secret".into()])
            .unwrap_err();
        assert!(error.to_string().contains("credential store"));
    }
}
