use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    net::IpAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::model::McpConfig;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HydianConfig {
    pub version: u32,
    pub listener: ListenerConfig,
    pub runtime: RuntimeConfig,
    pub naming: NamingConfig,
    pub restart: RestartConfig,
    pub logging: LoggingConfig,
    pub tui: TuiConfig,
    pub servers: ServersConfig,
    pub profiles: BTreeMap<String, ProfileConfig>,
    pub security: SecurityConfig,
    pub acknowledgements: AcknowledgementsConfig,
    pub exposure: ExposureConfig,
}

impl Default for HydianConfig {
    fn default() -> Self {
        Self {
            version: 1,
            listener: ListenerConfig::default(),
            runtime: RuntimeConfig::default(),
            naming: NamingConfig::default(),
            restart: RestartConfig::default(),
            logging: LoggingConfig::default(),
            tui: TuiConfig::default(),
            servers: ServersConfig::default(),
            profiles: BTreeMap::from([("default".into(), ProfileConfig::default())]),
            security: SecurityConfig::default(),
            acknowledgements: AcknowledgementsConfig::default(),
            exposure: ExposureConfig::default(),
        }
    }
}

impl HydianConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let input = fs::read_to_string(path)
            .with_context(|| format!("could not read configuration at {}", path.display()))?;
        Self::parse(&input)
            .with_context(|| format!("configuration is invalid at {}", path.display()))
    }

    pub fn parse(input: &str) -> Result<Self> {
        let config: Self = toml::from_str(input).context("could not parse config.toml")?;
        let issues = config.validate();
        if issues.iter().any(|issue| issue.level == IssueLevel::Error) {
            let rendered = issues
                .iter()
                .filter(|issue| issue.level == IssueLevel::Error)
                .map(|issue| format!("{}: {}", issue.field, issue.message))
                .collect::<Vec<_>>()
                .join("; ");
            bail!("{rendered}");
        }
        Ok(config)
    }

    #[must_use]
    pub fn validate(&self) -> Vec<ConfigIssue> {
        let mut issues = Vec::new();
        if self.version != 1 {
            issues.push(ConfigIssue::error(
                "version",
                "expected configuration version 1",
            ));
        }
        if self.listener.host.trim().is_empty() {
            issues.push(ConfigIssue::error(
                "listener.host",
                "listener host cannot be empty",
            ));
        }
        if self.listener.port == 0 {
            issues.push(ConfigIssue::error(
                "listener.port",
                "listener port must be between 1 and 65535",
            ));
        }
        if !self.listener.path.starts_with('/') {
            issues.push(ConfigIssue::error(
                "listener.path",
                "MCP endpoint path must begin with '/'",
            ));
        }
        if self.runtime.startup_timeout_seconds == 0 {
            issues.push(ConfigIssue::error(
                "runtime.startup_timeout_seconds",
                "startup timeout must be greater than zero",
            ));
        }
        if self.runtime.request_timeout_seconds == 0 {
            issues.push(ConfigIssue::error(
                "runtime.request_timeout_seconds",
                "request timeout must be greater than zero",
            ));
        }
        if self.runtime.shutdown_grace_seconds == 0 {
            issues.push(ConfigIssue::error(
                "runtime.shutdown_grace_seconds",
                "shutdown grace period must be greater than zero",
            ));
        }
        if self.naming.separator.is_empty()
            || !self
                .naming
                .separator
                .chars()
                .all(|character| matches!(character, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' | '.'))
        {
            issues.push(ConfigIssue::error(
                "naming.separator",
                "separator must use MCP tool-name characters",
            ));
        }
        if self.restart.enabled && self.restart.maximum_restarts_per_minute == 0 {
            issues.push(ConfigIssue::error(
                "restart.maximum_restarts_per_minute",
                "restart limit must be greater than zero when restart is enabled",
            ));
        }
        if self.servers.defaults.max_concurrent_calls == 0 {
            issues.push(ConfigIssue::error(
                "servers.defaults.max_concurrent_calls",
                "maximum concurrent calls must be greater than zero",
            ));
        }
        if !self.profiles.contains_key(&self.runtime.active_profile) {
            issues.push(ConfigIssue::error(
                "runtime.active_profile",
                format!(
                    "profile '{}' is not defined under [profiles]",
                    self.runtime.active_profile
                ),
            ));
        }
        for (name, profile) in &self.profiles {
            if profile.servers.is_empty() {
                issues.push(ConfigIssue::error(
                    format!("profiles.{name}.servers"),
                    "a profile must include at least one server name or '*'",
                ));
            }
        }
        if self.tui.refresh_rate_ms < 100 {
            issues.push(ConfigIssue::warning(
                "tui.refresh_rate_ms",
                "refresh rates below 100 ms add motion without useful operational detail",
            ));
        }
        if !is_loopback_host(&self.listener.host)
            && !self.acknowledgements.non_loopback_without_auth
        {
            issues.push(ConfigIssue::error(
                "acknowledgements.non_loopback_without_auth",
                format!(
                    "{} may expose plaintext MCP traffic without client authentication; set the acknowledgement only after reviewing `hydian explain non-loopback-without-auth`",
                    self.listener.host
                ),
            ));
        }
        if !self.security.validate_origin && !self.acknowledgements.disabled_origin_validation {
            issues.push(ConfigIssue::error(
                "acknowledgements.disabled_origin_validation",
                "origin validation is disabled without an explicit acknowledgement",
            ));
        }
        issues
    }

    #[must_use]
    pub fn endpoint(&self) -> String {
        let host = if self.listener.host.contains(':') && !self.listener.host.starts_with('[') {
            format!("[{}]", self.listener.host)
        } else {
            self.listener.host.clone()
        };
        format!("http://{host}:{}{}", self.listener.port, self.listener.path)
    }

    pub fn write(&self, path: &Path, backups: &Path) -> Result<WriteOutcome> {
        let rendered = toml::to_string_pretty(self).context("could not serialize config.toml")?;
        atomic_write_with_backup(path, rendered.as_bytes(), backups)
    }

    pub fn json_schema() -> Result<serde_json::Value> {
        serde_json::to_value(schemars::schema_for!(Self))
            .context("could not serialize Hydian configuration schema")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ListenerConfig {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 7337,
            path: "/mcp".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub active_profile: String,
    pub startup_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub shutdown_grace_seconds: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            active_profile: "default".into(),
            startup_timeout_seconds: 20,
            request_timeout_seconds: 120,
            shutdown_grace_seconds: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NamingConfig {
    pub separator: String,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            separator: "__".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RestartConfig {
    pub enabled: bool,
    pub initial_delay_ms: u64,
    pub maximum_delay_seconds: u64,
    pub maximum_restarts_per_minute: u32,
}

impl Default for RestartConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            initial_delay_ms: 500,
            maximum_delay_seconds: 30,
            maximum_restarts_per_minute: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: String,
    pub format: LogFormat,
    pub retain_days: u32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: LogFormat::Pretty,
            retain_days: 14,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LogFormat {
    Pretty,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct TuiConfig {
    pub enabled: bool,
    pub theme: TuiTheme,
    pub high_contrast: bool,
    pub symbols: SymbolMode,
    pub refresh_rate_ms: u64,
    pub mouse: bool,
    pub animations: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            theme: TuiTheme::System,
            high_contrast: false,
            symbols: SymbolMode::Unicode,
            refresh_rate_ms: 500,
            mouse: false,
            animations: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum TuiTheme {
    System,
    Dark,
    Light,
    HighContrast,
    Monochrome,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolMode {
    Unicode,
    Ascii,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ServersConfig {
    pub defaults: ServerDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ServerDefaults {
    pub max_concurrent_calls: usize,
}

impl Default for ServerDefaults {
    fn default() -> Self {
        Self {
            max_concurrent_calls: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileConfig {
    pub servers: Vec<String>,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            servers: vec!["*".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    pub validate_origin: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            validate_origin: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AcknowledgementsConfig {
    pub non_loopback_without_auth: bool,
    pub disabled_origin_validation: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ExposureConfig {
    pub active_provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigIssue {
    pub level: IssueLevel,
    pub field: String,
    pub message: String,
}

impl ConfigIssue {
    fn error(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: IssueLevel::Error,
            field: field.into(),
            message: message.into(),
        }
    }

    fn warning(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            level: IssueLevel::Warning,
            field: field.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WriteOutcome {
    pub changed_file: PathBuf,
    pub backup_file: Option<PathBuf>,
}

pub fn load_mcp_config(path: &Path) -> Result<McpConfig> {
    let input = fs::read_to_string(path)
        .with_context(|| format!("could not read MCP configuration at {}", path.display()))?;
    serde_json::from_str(&input)
        .with_context(|| format!("MCP configuration is invalid at {}", path.display()))
}

pub fn write_mcp_config(config: &McpConfig, path: &Path, backups: &Path) -> Result<WriteOutcome> {
    let rendered =
        serde_json::to_vec_pretty(config).context("could not serialize MCP configuration")?;
    atomic_write_with_backup(path, &rendered, backups)
}

pub fn atomic_write_with_backup(path: &Path, bytes: &[u8], backups: &Path) -> Result<WriteOutcome> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;
    fs::create_dir_all(backups)
        .with_context(|| format!("could not create {}", backups.display()))?;

    let backup_file = if path.exists() {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("configuration");
        let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let destination = backups.join(format!("{name}.{timestamp}.bak"));
        fs::copy(path, &destination).with_context(|| {
            format!(
                "could not back up {} to {}",
                path.display(),
                destination.display()
            )
        })?;
        Some(destination)
    } else {
        None
    };

    let mut temporary =
        NamedTempFile::new_in(parent).context("could not create temporary configuration file")?;
    temporary
        .write_all(bytes)
        .context("could not write temporary configuration file")?;
    temporary
        .as_file_mut()
        .sync_all()
        .context("could not flush temporary configuration file")?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("could not atomically replace {}", path.display()))?;

    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }

    Ok(WriteOutcome {
        changed_file: path.to_path_buf(),
        backup_file,
    })
}

#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let normalized = host.trim_matches(['[', ']']);
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::{
        HydianConfig, IssueLevel, atomic_write_with_backup, is_loopback_host, load_mcp_config,
    };
    use std::fs;

    #[test]
    fn default_config_round_trips() {
        let config = HydianConfig::default();
        let rendered = toml::to_string_pretty(&config).unwrap();
        let parsed = HydianConfig::parse(&rendered).unwrap();
        assert_eq!(parsed.endpoint(), "http://127.0.0.1:7337/mcp");
    }

    #[test]
    fn non_loopback_requires_acknowledgement() {
        let mut config = HydianConfig::default();
        config.listener.host = "0.0.0.0".into();
        assert!(config.validate().iter().any(|issue| {
            issue.level == IssueLevel::Error
                && issue.field == "acknowledgements.non_loopback_without_auth"
        }));
    }

    #[test]
    fn loopback_detection_handles_ipv4_ipv6_and_hostname() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));
        assert!(is_loopback_host("localhost"));
        assert!(!is_loopback_host("0.0.0.0"));
    }

    #[test]
    fn atomic_write_creates_backup() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("config.toml");
        let backups = directory.path().join("backups");
        fs::write(&target, b"old").unwrap();
        let outcome = atomic_write_with_backup(&target, b"new", &backups).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        let backup = outcome.backup_file.unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), "old");
    }

    #[test]
    fn mcp_configuration_round_trips_from_disk() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("mcp.json");
        fs::write(&target, r#"{"mcpServers":{}}"#).unwrap();
        assert!(load_mcp_config(&target).unwrap().servers.is_empty());
    }
}
