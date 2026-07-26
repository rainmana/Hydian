use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};
use serde::Serialize;

use crate::{
    config::{HydianConfig, WriteOutcome},
    model::McpConfig,
    paths::HydianPaths,
};

#[derive(Debug, Clone, Serialize)]
pub struct ProfileSummary {
    pub name: String,
    pub active: bool,
    pub configured_servers: Vec<String>,
    pub resolved_servers: Vec<String>,
}

pub fn list_profiles(config: &HydianConfig, mcp: &McpConfig) -> Result<Vec<ProfileSummary>> {
    config
        .profiles
        .iter()
        .map(|(name, profile)| {
            let resolved_servers = visible_server_names(config, mcp, Some(name))?;
            Ok(ProfileSummary {
                name: name.clone(),
                active: name == &config.runtime.active_profile,
                configured_servers: profile.servers.clone(),
                resolved_servers: resolved_servers.into_iter().collect(),
            })
        })
        .collect()
}

pub fn show_profile(config: &HydianConfig, mcp: &McpConfig, name: &str) -> Result<ProfileSummary> {
    let profile = config
        .profiles
        .get(name)
        .ok_or_else(|| anyhow!("profile `{name}` is not defined"))?;
    Ok(ProfileSummary {
        name: name.into(),
        active: name == config.runtime.active_profile,
        configured_servers: profile.servers.clone(),
        resolved_servers: visible_server_names(config, mcp, Some(name))?
            .into_iter()
            .collect(),
    })
}

pub fn use_profile(
    config: &mut HydianConfig,
    paths: &HydianPaths,
    name: &str,
    dry_run: bool,
) -> Result<Option<WriteOutcome>> {
    if !config.profiles.contains_key(name) {
        bail!(
            "profile `{name}` is not defined in {}",
            paths.config.display()
        );
    }
    config.runtime.active_profile = name.into();
    if dry_run {
        Ok(None)
    } else {
        config.write(&paths.config, &paths.backups).map(Some)
    }
}

pub fn visible_server_names(
    config: &HydianConfig,
    mcp: &McpConfig,
    profile_override: Option<&str>,
) -> Result<BTreeSet<String>> {
    let profile_name = profile_override.unwrap_or(&config.runtime.active_profile);
    let profile = config
        .profiles
        .get(profile_name)
        .ok_or_else(|| anyhow!("profile `{profile_name}` is not defined"))?;

    if profile.servers.iter().any(|name| name == "*") {
        return Ok(mcp
            .servers
            .iter()
            .filter(|(_, definition)| definition.enabled)
            .map(|(name, _)| name.clone())
            .collect());
    }

    let mut selected = BTreeSet::new();
    for name in &profile.servers {
        let definition = mcp.servers.get(name).ok_or_else(|| {
            anyhow!("profile `{profile_name}` references unknown server `{name}`")
        })?;
        if definition.enabled {
            selected.insert(name.clone());
        }
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        config::{HydianConfig, ProfileConfig},
        model::{McpConfig, McpServerDefinition},
    };

    use super::visible_server_names;

    #[test]
    fn wildcard_selects_only_enabled_servers() {
        let config = HydianConfig::default();
        let mcp = McpConfig {
            servers: BTreeMap::from([
                ("enabled".into(), McpServerDefinition::default()),
                (
                    "disabled".into(),
                    McpServerDefinition {
                        enabled: false,
                        ..Default::default()
                    },
                ),
            ]),
        };
        let names = visible_server_names(&config, &mcp, None).unwrap();
        assert_eq!(names.into_iter().collect::<Vec<_>>(), vec!["enabled"]);
    }

    #[test]
    fn named_profile_rejects_unknown_server() {
        let mut config = HydianConfig::default();
        config.profiles.insert(
            "broken".into(),
            ProfileConfig {
                servers: vec!["missing".into()],
            },
        );
        let error =
            visible_server_names(&config, &McpConfig::default(), Some("broken")).unwrap_err();
        assert!(error.to_string().contains("unknown server"));
    }
}
