use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use tokio::process::Command;

use crate::{config::atomic_write_with_backup, paths::HydianPaths};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServicePlatform {
    Windows,
    Linux,
    Macos,
}

impl ServicePlatform {
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServicePlan {
    pub platform: ServicePlatform,
    pub system: bool,
    pub executable: PathBuf,
    pub hydian_home: PathBuf,
    pub definition_path: PathBuf,
    pub definition: String,
    pub install_commands: Vec<Vec<String>>,
    pub uninstall_commands: Vec<Vec<String>>,
    pub start_command: Vec<String>,
    pub stop_command: Vec<String>,
    pub status_command: Vec<String>,
    pub configuration_path: PathBuf,
    pub log_path: PathBuf,
    pub semantics: String,
}

pub fn plan(
    platform: ServicePlatform,
    system: bool,
    executable: &Path,
    paths: &HydianPaths,
) -> Result<ServicePlan> {
    let executable = executable
        .canonicalize()
        .with_context(|| format!("could not resolve executable {}", executable.display()))?;
    let executable = normalize_external_path(&executable);
    let executable_text = executable.to_string_lossy();
    let home_text = paths.home.to_string_lossy();
    match platform {
        ServicePlatform::Windows => {
            if system {
                bail!(
                    "Windows system-service mode is not implemented in v0.1; the default Scheduled Task runs at current-user logon without storing a password"
                );
            }
            let definition_path = paths.home.join("service").join("hydian-task.xml");
            let definition = windows_task_xml(&executable_text, &home_text);
            Ok(ServicePlan {
                platform,
                system,
                executable,
                hydian_home: paths.home.clone(),
                definition_path: definition_path.clone(),
                definition,
                install_commands: vec![vec![
                    "schtasks.exe".into(),
                    "/Create".into(),
                    "/TN".into(),
                    "Hydian".into(),
                    "/XML".into(),
                    definition_path.to_string_lossy().into_owned(),
                    "/F".into(),
                ]],
                uninstall_commands: vec![vec![
                    "schtasks.exe".into(),
                    "/Delete".into(),
                    "/TN".into(),
                    "Hydian".into(),
                    "/F".into(),
                ]],
                start_command: vec![
                    "schtasks.exe".into(),
                    "/Run".into(),
                    "/TN".into(),
                    "Hydian".into(),
                ],
                stop_command: vec![
                    "schtasks.exe".into(),
                    "/End".into(),
                    "/TN".into(),
                    "Hydian".into(),
                ],
                status_command: vec![
                    "schtasks.exe".into(),
                    "/Query".into(),
                    "/TN".into(),
                    "Hydian".into(),
                    "/FO".into(),
                    "LIST".into(),
                ],
                configuration_path: paths.config.clone(),
                log_path: paths.logs.clone(),
                semantics: "Current-user Scheduled Task triggered at logon; no password is requested or stored.".into(),
            })
        }
        ServicePlatform::Linux => {
            let definition_path = if system {
                PathBuf::from("/etc/systemd/system/hydian.service")
            } else {
                directories::BaseDirs::new()
                    .ok_or_else(|| anyhow!("user directories are unavailable"))?
                    .config_dir()
                    .join("systemd/user/hydian.service")
            };
            let definition = format!(
                "[Unit]\nDescription=Hydian MCP multiplexer\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart={} --home {} serve\nRestart=on-failure\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
                systemd_escape(&executable_text),
                systemd_escape(&home_text)
            );
            let prefix = if system {
                vec![]
            } else {
                vec!["--user".into()]
            };
            let command = |action: &str| {
                let mut parts = vec!["systemctl".into()];
                parts.extend(prefix.clone());
                parts.extend([action.into(), "hydian.service".into()]);
                parts
            };
            let mut daemon_reload = vec!["systemctl".into()];
            daemon_reload.extend(prefix.clone());
            daemon_reload.push("daemon-reload".into());
            Ok(ServicePlan {
                platform,
                system,
                executable,
                hydian_home: paths.home.clone(),
                definition_path,
                definition,
                install_commands: vec![daemon_reload.clone(), command("enable")],
                uninstall_commands: vec![command("disable"), daemon_reload],
                start_command: command("start"),
                stop_command: command("stop"),
                status_command: command("status"),
                configuration_path: paths.config.clone(),
                log_path: paths.logs.clone(),
                semantics: if system {
                    "System-level systemd unit; installation normally requires elevated file permissions.".into()
                } else {
                    "User-level systemd unit; no root privileges are required.".into()
                },
            })
        }
        ServicePlatform::Macos => {
            let definition_path = if system {
                PathBuf::from("/Library/LaunchDaemons/io.hydian.gateway.plist")
            } else {
                directories::BaseDirs::new()
                    .ok_or_else(|| anyhow!("user directories are unavailable"))?
                    .home_dir()
                    .join("Library/LaunchAgents/io.hydian.gateway.plist")
            };
            let definition = launchd_plist(&executable_text, &home_text, paths);
            let domain = if system { "system" } else { "gui/$UID" };
            Ok(ServicePlan {
                platform,
                system,
                executable,
                hydian_home: paths.home.clone(),
                definition_path: definition_path.clone(),
                definition,
                install_commands: vec![vec![
                    "launchctl".into(),
                    "bootstrap".into(),
                    domain.into(),
                    definition_path.to_string_lossy().into_owned(),
                ]],
                uninstall_commands: vec![vec![
                    "launchctl".into(),
                    "bootout".into(),
                    format!("{domain}/io.hydian.gateway"),
                ]],
                start_command: vec![
                    "launchctl".into(),
                    "kickstart".into(),
                    format!("{domain}/io.hydian.gateway"),
                ],
                stop_command: vec![
                    "launchctl".into(),
                    "kill".into(),
                    "SIGTERM".into(),
                    format!("{domain}/io.hydian.gateway"),
                ],
                status_command: vec![
                    "launchctl".into(),
                    "print".into(),
                    format!("{domain}/io.hydian.gateway"),
                ],
                configuration_path: paths.config.clone(),
                log_path: paths.logs.clone(),
                semantics: if system {
                    "System LaunchDaemon; installation requires privileged file access.".into()
                } else {
                    "Per-user LaunchAgent loaded in the graphical login domain.".into()
                },
            })
        }
    }
}

pub fn current_plan(system: bool, paths: &HydianPaths) -> Result<ServicePlan> {
    plan(
        ServicePlatform::current(),
        system,
        &std::env::current_exe().context("could not locate the current Hydian executable")?,
        paths,
    )
}

pub async fn install(plan: &ServicePlan) -> Result<()> {
    let parent = plan
        .definition_path
        .parent()
        .ok_or_else(|| anyhow!("service definition has no parent directory"))?;
    fs::create_dir_all(parent)?;
    atomic_write_with_backup(
        &plan.definition_path,
        plan.definition.as_bytes(),
        &plan.hydian_home.join("backups"),
    )?;
    for command in &plan.install_commands {
        execute(command).await?;
    }
    execute(&plan.status_command).await?;
    Ok(())
}

pub async fn uninstall(plan: &ServicePlan) -> Result<()> {
    for command in &plan.uninstall_commands {
        let _ = execute(command).await;
    }
    if plan.definition_path.exists() {
        fs::remove_file(&plan.definition_path)?;
    }
    Ok(())
}

pub async fn execute(command: &[String]) -> Result<String> {
    let (program, arguments) = command
        .split_first()
        .ok_or_else(|| anyhow!("service command is empty"))?;
    let uid = if arguments.iter().any(|argument| argument.contains("$UID")) {
        current_uid()
    } else {
        String::new()
    };
    let arguments = arguments
        .iter()
        .map(|argument| argument.replace("$UID", &uid))
        .collect::<Vec<_>>();
    let output = Command::new(program)
        .args(&arguments)
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("could not execute `{}`", display_command(command)))?;
    if !output.status.success() {
        bail!(
            "service command failed: {}\n{}",
            display_command(command),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[must_use]
pub fn display_command(command: &[String]) -> String {
    command
        .iter()
        .map(|part| {
            if part.contains([' ', '"']) {
                format!("\"{}\"", part.replace('"', "\\\""))
            } else {
                part.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn is_temporary_path(path: &Path) -> bool {
    std::env::temp_dir()
        .canonicalize()
        .ok()
        .zip(path.canonicalize().ok())
        .is_some_and(|(temporary, executable)| executable.starts_with(temporary))
}

fn windows_task_xml(executable: &str, home: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><RestartOnFailure><Interval>PT1M</Interval><Count>5</Count></RestartOnFailure><ExecutionTimeLimit>PT0S</ExecutionTimeLimit></Settings>
  <Actions Context="Author"><Exec><Command>{}</Command><Arguments>--home &quot;{}&quot; serve</Arguments></Exec></Actions>
</Task>
"#,
        xml_escape(executable),
        xml_escape(home)
    )
}

fn normalize_external_path(path: &Path) -> PathBuf {
    let rendered = path.to_string_lossy();
    if let Some(rest) = rendered.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    rendered
        .strip_prefix(r"\\?\")
        .map_or_else(|| path.to_path_buf(), PathBuf::from)
}

fn current_uid() -> String {
    std::env::var("UID").unwrap_or_else(|_| {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_else(|| "501".into())
    })
}

fn launchd_plist(executable: &str, home: &str, paths: &HydianPaths) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>io.hydian.gateway</string>
<key>ProgramArguments</key><array><string>{}</string><string>--home</string><string>{}</string><string>serve</string></array>
<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>
<key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string>
</dict></plist>
"#,
        xml_escape(executable),
        xml_escape(home),
        xml_escape(&paths.logs.join("service.stdout.log").to_string_lossy()),
        xml_escape(&paths.logs.join("service.stderr.log").to_string_lossy())
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn systemd_escape(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::{ServicePlatform, plan};
    use crate::paths::HydianPaths;
    use tempfile::TempDir;

    #[test]
    fn dry_run_plans_cover_every_platform_without_accounts_or_privileges() {
        let directory = TempDir::new().unwrap();
        let executable = std::env::current_exe().unwrap();
        let paths = HydianPaths::resolve(Some(directory.path()), None, None).unwrap();
        for platform in [
            ServicePlatform::Windows,
            ServicePlatform::Linux,
            ServicePlatform::Macos,
        ] {
            let plan = plan(platform, false, &executable, &paths).unwrap();
            assert!(plan.definition.contains("hydian"));
            assert!(!plan.install_commands.is_empty());
            assert!(plan.definition.contains("--home"));
            assert_eq!(plan.hydian_home, paths.home);
        }
    }

    #[test]
    fn windows_task_uses_utf8_and_an_external_command_path() {
        let directory = TempDir::new().unwrap();
        let executable = std::env::current_exe().unwrap();
        let paths = HydianPaths::resolve(Some(directory.path()), None, None).unwrap();
        let plan = plan(ServicePlatform::Windows, false, &executable, &paths).unwrap();
        assert!(
            plan.definition
                .starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>")
        );
        assert!(!plan.executable.to_string_lossy().starts_with(r"\\?\"));
        assert!(plan.semantics.contains("no password"));
    }

    #[test]
    fn windows_system_mode_is_not_overclaimed() {
        let directory = TempDir::new().unwrap();
        let executable = std::env::current_exe().unwrap();
        let paths = HydianPaths::resolve(Some(directory.path()), None, None).unwrap();
        let error = plan(ServicePlatform::Windows, true, &executable, &paths).unwrap_err();
        assert!(error.to_string().contains("not implemented"));
    }
}
