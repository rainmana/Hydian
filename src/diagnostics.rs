use std::path::Path;

use serde::Serialize;

use crate::{
    config::{HydianConfig, IssueLevel, load_mcp_config},
    model::BackendTransport,
    paths::HydianPaths,
    secrets::literal_header_names,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticStatus {
    Ready,
    Warning,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticCheck {
    pub status: DiagnosticStatus,
    pub name: String,
    pub reason: String,
    pub affected: String,
    pub configuration: Option<String>,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub ready: bool,
    pub strict: bool,
    pub checks: Vec<DiagnosticCheck>,
}

impl DoctorReport {
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == DiagnosticStatus::Failed)
    }

    #[must_use]
    pub fn has_warnings(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == DiagnosticStatus::Warning)
    }
}

#[must_use]
pub fn run(paths: &HydianPaths, strict: bool) -> DoctorReport {
    let mut checks = Vec::new();
    check_directory(paths, &mut checks);

    let config = check_hydian_config(paths, &mut checks);
    let mcp = check_mcp_config(paths, &mut checks);

    if let (Some(config), Some(mcp)) = (&config, &mcp) {
        for (name, definition) in &mcp.servers {
            match definition.transport() {
                Some(BackendTransport::Stdio) => {
                    if let Some(command) = &definition.command {
                        if which::which(command).is_ok() {
                            checks.push(DiagnosticCheck {
                                status: DiagnosticStatus::Ready,
                                name: format!("server.{name}.executable"),
                                reason: format!("found executable `{command}`"),
                                affected: format!("stdio backend `{name}`"),
                                configuration: Some(paths.mcp_config.display().to_string()),
                                fix: None,
                            });
                        } else {
                            checks.push(DiagnosticCheck {
                                status: DiagnosticStatus::Warning,
                                name: format!("server.{name}.executable"),
                                reason: format!("executable `{command}` was not found on PATH"),
                                affected: format!(
                                    "tools from `{name}` will be unavailable until it starts"
                                ),
                                configuration: Some(paths.mcp_config.display().to_string()),
                                fix: Some(
                                    "install the executable or update the server command".into(),
                                ),
                            });
                        }
                    }
                }
                Some(BackendTransport::StreamableHttp) => {
                    if let Some(url) = &definition.url {
                        match reqwest::Url::parse(url) {
                            Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => {
                                checks.push(DiagnosticCheck {
                                    status: DiagnosticStatus::Ready,
                                    name: format!("server.{name}.url"),
                                    reason: "remote URL is syntactically valid".into(),
                                    affected: format!("Streamable HTTP backend `{name}`"),
                                    configuration: Some(paths.mcp_config.display().to_string()),
                                    fix: None,
                                });
                            }
                            _ => checks.push(DiagnosticCheck {
                                status: DiagnosticStatus::Failed,
                                name: format!("server.{name}.url"),
                                reason: "remote URL is not a valid HTTP or HTTPS URL".into(),
                                affected: format!("Streamable HTTP backend `{name}`"),
                                configuration: Some(paths.mcp_config.display().to_string()),
                                fix: Some("set `url` to the backend's MCP endpoint".into()),
                            }),
                        }
                    }
                }
                None => checks.push(DiagnosticCheck {
                    status: DiagnosticStatus::Failed,
                    name: format!("server.{name}.transport"),
                    reason: "server has neither a recognized stdio nor Streamable HTTP definition"
                        .into(),
                    affected: format!("backend `{name}`"),
                    configuration: Some(paths.mcp_config.display().to_string()),
                    fix: Some("add `command` for stdio or `url` for Streamable HTTP".into()),
                }),
            }

            let literal_headers = literal_header_names(&definition.headers);
            if !literal_headers.is_empty() {
                checks.push(DiagnosticCheck {
                    status: DiagnosticStatus::Warning,
                    name: format!("server.{name}.literal_secrets"),
                    reason: format!(
                        "likely secret headers use literal values: {}",
                        literal_headers.join(", ")
                    ),
                    affected: format!("credentials for `{name}` are stored directly in mcp.json"),
                    configuration: Some(paths.mcp_config.display().to_string()),
                    fix: Some("replace each value with `env:VARIABLE` or `file:path`".into()),
                });
            }
        }

        let visible = crate::profiles::visible_server_names(
            config,
            mcp,
            Some(&config.runtime.active_profile),
        );
        match visible {
            Ok(names) => checks.push(DiagnosticCheck {
                status: DiagnosticStatus::Ready,
                name: "profile.active".into(),
                reason: format!(
                    "profile `{}` selects {} configured server(s)",
                    config.runtime.active_profile,
                    names.len()
                ),
                affected: "tool catalog".into(),
                configuration: Some(paths.config.display().to_string()),
                fix: None,
            }),
            Err(error) => checks.push(DiagnosticCheck {
                status: DiagnosticStatus::Failed,
                name: "profile.active".into(),
                reason: error.to_string(),
                affected: "tool catalog".into(),
                configuration: Some(paths.config.display().to_string()),
                fix: Some("correct the active profile and its server names".into()),
            }),
        }
    }

    let has_failures = checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Failed);
    let has_warnings = checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Warning);
    DoctorReport {
        ready: !(has_failures || strict && has_warnings),
        strict,
        checks,
    }
}

fn check_directory(paths: &HydianPaths, checks: &mut Vec<DiagnosticCheck>) {
    if paths.home.exists() && paths.home.is_dir() {
        checks.push(DiagnosticCheck {
            status: DiagnosticStatus::Ready,
            name: "paths.home".into(),
            reason: "Hydian home directory exists".into(),
            affected: "configuration, logs, backups, and runtime status".into(),
            configuration: Some(paths.home.display().to_string()),
            fix: None,
        });
    } else {
        checks.push(DiagnosticCheck {
            status: DiagnosticStatus::Failed,
            name: "paths.home".into(),
            reason: "Hydian home directory does not exist".into(),
            affected: "all persistent Hydian files".into(),
            configuration: Some(paths.home.display().to_string()),
            fix: Some("run `hydian init`".into()),
        });
    }
}

fn check_hydian_config(
    paths: &HydianPaths,
    checks: &mut Vec<DiagnosticCheck>,
) -> Option<HydianConfig> {
    match HydianConfig::load(&paths.config) {
        Ok(config) => {
            for issue in config.validate() {
                checks.push(DiagnosticCheck {
                    status: match issue.level {
                        IssueLevel::Warning => DiagnosticStatus::Warning,
                        IssueLevel::Error => DiagnosticStatus::Failed,
                    },
                    name: format!("config.{}", issue.field),
                    reason: issue.message,
                    affected: "Hydian startup and request handling".into(),
                    configuration: Some(paths.config.display().to_string()),
                    fix: Some(format!("update `{}`", issue.field)),
                });
            }
            checks.push(DiagnosticCheck {
                status: DiagnosticStatus::Ready,
                name: "config.toml".into(),
                reason: "Hydian configuration loaded successfully".into(),
                affected: "gateway runtime".into(),
                configuration: Some(paths.config.display().to_string()),
                fix: None,
            });
            Some(config)
        }
        Err(error) => {
            checks.push(config_failure(
                "config.toml",
                &paths.config,
                error.to_string(),
                "run `hydian init` or correct the reported field",
            ));
            None
        }
    }
}

fn check_mcp_config(
    paths: &HydianPaths,
    checks: &mut Vec<DiagnosticCheck>,
) -> Option<crate::model::McpConfig> {
    match load_mcp_config(&paths.mcp_config) {
        Ok(config) => {
            checks.push(DiagnosticCheck {
                status: DiagnosticStatus::Ready,
                name: "mcp.json".into(),
                reason: format!("loaded {} configured server(s)", config.servers.len()),
                affected: "backend registry".into(),
                configuration: Some(paths.mcp_config.display().to_string()),
                fix: None,
            });
            Some(config)
        }
        Err(error) => {
            checks.push(config_failure(
                "mcp.json",
                &paths.mcp_config,
                error.to_string(),
                "run `hydian init`, import a configuration, or correct the JSON",
            ));
            None
        }
    }
}

fn config_failure(name: &str, path: &Path, reason: String, fix: &str) -> DiagnosticCheck {
    DiagnosticCheck {
        status: DiagnosticStatus::Failed,
        name: name.into(),
        reason,
        affected: "gateway startup".into(),
        configuration: Some(path.display().to_string()),
        fix: Some(fix.into()),
    }
}
