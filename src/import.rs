use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as DeError, MapAccess, Visitor},
};
use serde_json::Value;

use crate::{
    cli::{ConflictChoice, ImportFormat},
    config::{WriteOutcome, load_mcp_config, write_mcp_config},
    model::{McpConfig, McpServerDefinition},
};

#[derive(Debug, Clone)]
pub struct ImportedServers {
    pub format: DetectedFormat,
    pub servers: BTreeMap<String, McpServerDefinition>,
    pub root_unknown_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectedFormat {
    Claude,
    Vscode,
    Cursor,
    Codex,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPlan {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub format: DetectedFormat,
    pub entries: Vec<ImportEntry>,
    pub root_unknown_fields: Vec<String>,
    pub resulting_server_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportEntry {
    pub source_name: String,
    pub destination_name: String,
    pub action: ImportAction,
    pub reason: String,
    pub unknown_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportAction {
    Added,
    Changed,
    Skipped,
    Conflicted,
}

pub struct PlannedImport {
    pub plan: ImportPlan,
    pub result: McpConfig,
}

pub fn read_import(path: &Path, requested: ImportFormat) -> Result<ImportedServers> {
    let input = fs::read_to_string(path)
        .with_context(|| format!("could not read import source {}", path.display()))?;
    let selected = match requested {
        ImportFormat::Auto => {
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
            {
                ImportFormat::Codex
            } else {
                ImportFormat::Auto
            }
        }
        other => other,
    };

    match selected {
        ImportFormat::Codex => parse_codex(&input),
        ImportFormat::Auto | ImportFormat::Claude | ImportFormat::Vscode | ImportFormat::Cursor => {
            parse_json(&input, selected)
        }
    }
}

pub fn plan_import(
    source: &Path,
    destination: &Path,
    imported: ImportedServers,
    conflict: ConflictChoice,
    rename_suffix: &str,
) -> Result<PlannedImport> {
    if rename_suffix.is_empty() {
        bail!("rename suffix cannot be empty");
    }

    let mut result = if destination.exists() {
        load_mcp_config(destination)?
    } else {
        McpConfig::default()
    };
    let mut entries = Vec::new();

    for (name, definition) in imported.servers {
        let unknown_fields = definition.unknown_field_names();
        match result.servers.get(&name) {
            None => {
                result.servers.insert(name.clone(), definition);
                entries.push(ImportEntry {
                    source_name: name.clone(),
                    destination_name: name,
                    action: ImportAction::Added,
                    reason: "server does not exist in Hydian".into(),
                    unknown_fields,
                });
            }
            Some(existing) if existing == &definition => entries.push(ImportEntry {
                source_name: name.clone(),
                destination_name: name,
                action: ImportAction::Skipped,
                reason: "identical server already exists".into(),
                unknown_fields,
            }),
            Some(_) => match conflict {
                ConflictChoice::Skip => entries.push(ImportEntry {
                    source_name: name.clone(),
                    destination_name: name,
                    action: ImportAction::Conflicted,
                    reason: "different server already uses this name; selected choice is skip"
                        .into(),
                    unknown_fields,
                }),
                ConflictChoice::Replace => {
                    result.servers.insert(name.clone(), definition);
                    entries.push(ImportEntry {
                        source_name: name.clone(),
                        destination_name: name,
                        action: ImportAction::Changed,
                        reason: "existing server will be replaced".into(),
                        unknown_fields,
                    });
                }
                ConflictChoice::Rename => {
                    let destination_name =
                        next_available_name(&result.servers, &name, rename_suffix);
                    result.servers.insert(destination_name.clone(), definition);
                    entries.push(ImportEntry {
                        source_name: name,
                        destination_name,
                        action: ImportAction::Added,
                        reason: "conflicting server will be imported under a deterministic name"
                            .into(),
                        unknown_fields,
                    });
                }
            },
        }
    }

    let plan = ImportPlan {
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        format: imported.format,
        entries,
        root_unknown_fields: imported.root_unknown_fields,
        resulting_server_count: result.servers.len(),
    };
    Ok(PlannedImport { plan, result })
}

pub fn apply_import(planned: &PlannedImport, backups: &Path) -> Result<WriteOutcome> {
    write_mcp_config(&planned.result, &planned.plan.destination, backups)
}

fn parse_json(input: &str, requested: ImportFormat) -> Result<ImportedServers> {
    let root: JsonImportRoot = serde_json::from_str(input)
        .map_err(|error| anyhow!("could not parse JSON import source: {error}"))?;

    let (format, server_map) = match requested {
        ImportFormat::Claude => (
            DetectedFormat::Claude,
            root.mcp_servers
                .ok_or_else(|| anyhow!("expected an `mcpServers` object"))?,
        ),
        ImportFormat::Cursor => (
            DetectedFormat::Cursor,
            root.mcp_servers
                .ok_or_else(|| anyhow!("expected an `mcpServers` object"))?,
        ),
        ImportFormat::Vscode => (
            DetectedFormat::Vscode,
            root.servers
                .ok_or_else(|| anyhow!("expected a `servers` object"))?,
        ),
        ImportFormat::Auto => match (root.mcp_servers, root.servers) {
            (Some(servers), None) => (DetectedFormat::Claude, servers),
            (None, Some(servers)) => (DetectedFormat::Vscode, servers),
            (Some(_), Some(_)) => {
                bail!("source contains both `mcpServers` and `servers`; select a format explicitly")
            }
            (None, None) => bail!("source has neither an `mcpServers` nor a `servers` object"),
        },
        ImportFormat::Codex => unreachable!("Codex imports are parsed as TOML"),
    };

    let servers = server_map
        .0
        .into_iter()
        .map(|(name, value)| {
            let definition: McpServerDefinition = serde_json::from_value(value)
                .with_context(|| format!("server `{name}` is not a valid definition"))?;
            validate_server(&name, &definition)?;
            Ok((name, definition))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;

    Ok(ImportedServers {
        format,
        servers,
        root_unknown_fields: root.extra.into_keys().collect(),
    })
}

fn parse_codex(input: &str) -> Result<ImportedServers> {
    let mut root: toml::Table =
        toml::from_str(input).context("could not parse Codex TOML import source")?;
    let raw_servers = root
        .remove("mcp_servers")
        .ok_or_else(|| anyhow!("expected a `[mcp_servers]` table"))?;
    let server_table = raw_servers
        .as_table()
        .ok_or_else(|| anyhow!("`mcp_servers` must be a table"))?;

    let mut servers = BTreeMap::new();
    for (name, raw_definition) in server_table {
        let table = raw_definition
            .as_table()
            .ok_or_else(|| anyhow!("`mcp_servers.{name}` must be a table"))?;
        let mut remaining = table.clone();
        let command = take_string(&mut remaining, "command")?;
        let args = take_string_array(&mut remaining, "args")?;
        let env = take_string_table(&mut remaining, "env")?;
        let cwd = take_string(&mut remaining, "cwd")?;
        let url = take_string(&mut remaining, "url")?;
        let mut headers = take_string_table(&mut remaining, "http_headers")?;
        for (header, variable) in take_string_table(&mut remaining, "env_http_headers")? {
            headers.insert(header, format!("env:{variable}"));
        }
        if let Some(variable) = take_string(&mut remaining, "bearer_token_env_var")? {
            headers
                .entry("Authorization".into())
                .or_insert_with(|| format!("env:{variable}"));
        }
        let startup_timeout_seconds = take_seconds(&mut remaining, "startup_timeout_sec")?;
        let request_timeout_seconds = take_seconds(&mut remaining, "tool_timeout_sec")?;
        let enabled = take_bool(&mut remaining, "enabled")?.unwrap_or(true);

        let extra = remaining
            .into_iter()
            .map(|(key, value)| {
                serde_json::to_value(value)
                    .map(|converted| (key, converted))
                    .context("could not preserve an unrecognized Codex field")
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let definition = McpServerDefinition {
            kind: Some(if url.is_some() {
                "streamable-http".into()
            } else {
                "stdio".into()
            }),
            command,
            args,
            env,
            cwd,
            url,
            headers,
            startup_timeout_seconds,
            request_timeout_seconds,
            enabled,
            extra,
        };
        validate_server(name, &definition)?;
        servers.insert(name.clone(), definition);
    }

    Ok(ImportedServers {
        format: DetectedFormat::Codex,
        servers,
        root_unknown_fields: root.into_iter().map(|(key, _)| key).collect(),
    })
}

fn validate_server(name: &str, definition: &McpServerDefinition) -> Result<()> {
    match definition.transport() {
        Some(crate::model::BackendTransport::Stdio) if definition.command.is_none() => {
            bail!("stdio server `{name}` has no command")
        }
        Some(crate::model::BackendTransport::StreamableHttp) if definition.url.is_none() => {
            bail!("Streamable HTTP server `{name}` has no URL")
        }
        Some(_) => Ok(()),
        None => bail!("server `{name}` has no recognized transport"),
    }
}

fn next_available_name(
    servers: &BTreeMap<String, McpServerDefinition>,
    original: &str,
    suffix: &str,
) -> String {
    let base = format!("{original}-{suffix}");
    if !servers.contains_key(&base) {
        return base;
    }
    let mut index = 2_u64;
    loop {
        let candidate = format!("{base}-{index}");
        if !servers.contains_key(&candidate) {
            return candidate;
        }
        index = index
            .checked_add(1)
            .expect("server rename suffix space was exhausted");
    }
}

fn take_string(table: &mut toml::Table, key: &str) -> Result<Option<String>> {
    table
        .remove(key)
        .map(|value| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow!("`{key}` must be a string"))
        })
        .transpose()
}

fn take_bool(table: &mut toml::Table, key: &str) -> Result<Option<bool>> {
    table
        .remove(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| anyhow!("`{key}` must be a boolean"))
        })
        .transpose()
}

fn take_string_array(table: &mut toml::Table, key: &str) -> Result<Vec<String>> {
    let Some(value) = table.remove(key) else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| anyhow!("`{key}` must be an array"))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow!("every `{key}` item must be a string"))
        })
        .collect()
}

fn take_string_table(table: &mut toml::Table, key: &str) -> Result<BTreeMap<String, String>> {
    let Some(value) = table.remove(key) else {
        return Ok(BTreeMap::new());
    };
    value
        .as_table()
        .ok_or_else(|| anyhow!("`{key}` must be a table"))?
        .iter()
        .map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), value.to_owned()))
                .ok_or_else(|| anyhow!("every `{key}` value must be a string"))
        })
        .collect()
}

fn take_seconds(table: &mut toml::Table, key: &str) -> Result<Option<u64>> {
    let Some(value) = table.remove(key) else {
        return Ok(None);
    };
    if let Some(integer) = value.as_integer() {
        return u64::try_from(integer)
            .map(Some)
            .map_err(|_| anyhow!("`{key}` cannot be negative"));
    }
    if let Some(float) = value.as_float() {
        #[allow(clippy::cast_precision_loss)]
        let maximum = u64::MAX as f64;
        if float.is_sign_negative() || !float.is_finite() || float.ceil() > maximum {
            bail!("`{key}` must be a finite, non-negative number");
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let seconds = float.ceil() as u64;
        return Ok(Some(seconds));
    }
    bail!("`{key}` must be a number")
}

#[derive(Debug, Deserialize)]
struct JsonImportRoot {
    #[serde(rename = "mcpServers")]
    mcp_servers: Option<UniqueServerMap>,
    servers: Option<UniqueServerMap>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Debug)]
struct UniqueServerMap(BTreeMap<String, Value>);

impl<'de> Deserialize<'de> for UniqueServerMap {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UniqueMapVisitor;

        impl<'de> Visitor<'de> for UniqueMapVisitor {
            type Value = UniqueServerMap;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object containing uniquely named MCP servers")
            }

            fn visit_map<M>(self, mut access: M) -> std::result::Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut servers = BTreeMap::new();
                while let Some((name, value)) = access.next_entry::<String, Value>()? {
                    if servers.insert(name.clone(), value).is_some() {
                        return Err(M::Error::custom(format!("duplicate server name `{name}`")));
                    }
                }
                Ok(UniqueServerMap(servers))
            }
        }

        deserializer.deserialize_map(UniqueMapVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::{DetectedFormat, ImportAction, parse_codex, parse_json, plan_import};
    use crate::{
        cli::{ConflictChoice, ImportFormat},
        model::{McpConfig, McpServerDefinition},
    };
    use std::{collections::BTreeMap, fs};

    #[test]
    fn parses_claude_shape() {
        let imported = parse_json(
            r#"{"mcpServers":{"filesystem":{"command":"npx","args":["-y"]}}}"#,
            ImportFormat::Auto,
        )
        .unwrap();
        assert!(matches!(imported.format, DetectedFormat::Claude));
        assert_eq!(
            imported.servers["filesystem"].command.as_deref(),
            Some("npx")
        );
    }

    #[test]
    fn parses_vscode_shape() {
        let imported = parse_json(
            r#"{"servers":{"filesystem":{"type":"stdio","command":"npx"}}}"#,
            ImportFormat::Auto,
        )
        .unwrap();
        assert!(matches!(imported.format, DetectedFormat::Vscode));
        assert!(imported.servers.contains_key("filesystem"));
    }

    #[test]
    fn rejects_duplicate_server_names() {
        let error = parse_json(
            r#"{"mcpServers":{"same":{"command":"one"},"same":{"command":"two"}}}"#,
            ImportFormat::Auto,
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate server name"));
    }

    #[test]
    fn parses_current_codex_toml_shape() {
        let imported = parse_codex(
            r#"
[mcp_servers.docs]
url = "https://example.test/mcp"
bearer_token_env_var = "DOCS_TOKEN"
startup_timeout_sec = 5.5
"#,
        )
        .unwrap();
        let docs = &imported.servers["docs"];
        assert_eq!(docs.url.as_deref(), Some("https://example.test/mcp"));
        assert_eq!(docs.headers["Authorization"], "env:DOCS_TOKEN");
        assert_eq!(docs.startup_timeout_seconds, Some(6));
    }

    #[test]
    fn deterministic_rename_preserves_both_definitions() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("mcp.json");
        let existing = McpConfig {
            servers: BTreeMap::from([(
                "docs".into(),
                McpServerDefinition {
                    command: Some("old".into()),
                    ..Default::default()
                },
            )]),
        };
        fs::write(&destination, serde_json::to_vec(&existing).unwrap()).unwrap();
        let imported = parse_json(
            r#"{"mcpServers":{"docs":{"command":"new"}}}"#,
            ImportFormat::Auto,
        )
        .unwrap();
        let planned = plan_import(
            directory.path().join("source.json").as_path(),
            &destination,
            imported,
            ConflictChoice::Rename,
            "imported",
        )
        .unwrap();
        assert_eq!(planned.plan.entries[0].action, ImportAction::Added);
        assert!(planned.result.servers.contains_key("docs-imported"));
    }
}
