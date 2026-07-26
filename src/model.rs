use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BackendTransport {
    Stdio,
    StreamableHttp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerDefinition {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_timeout_seconds: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_timeout_seconds: Option<u64>,

    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl McpServerDefinition {
    #[must_use]
    pub fn transport(&self) -> Option<BackendTransport> {
        match self.kind.as_deref() {
            Some("stdio") => Some(BackendTransport::Stdio),
            Some("streamable-http" | "streamable_http" | "http") => {
                Some(BackendTransport::StreamableHttp)
            }
            None if self.url.is_some() => Some(BackendTransport::StreamableHttp),
            None if self.command.is_some() => Some(BackendTransport::Stdio),
            Some(_) | None => None,
        }
    }

    #[must_use]
    pub fn unknown_field_names(&self) -> Vec<String> {
        self.extra.keys().cloned().collect()
    }
}

impl Default for McpServerDefinition {
    fn default() -> Self {
        Self {
            kind: None,
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
            url: None,
            headers: BTreeMap::new(),
            startup_timeout_seconds: None,
            request_timeout_seconds: None,
            enabled: true,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(rename = "mcpServers")]
    pub servers: BTreeMap<String, McpServerDefinition>,
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{BackendTransport, McpServerDefinition};

    #[test]
    fn infers_transports_from_primary_fields() {
        let stdio = McpServerDefinition {
            command: Some("server".into()),
            ..Default::default()
        };
        let http = McpServerDefinition {
            url: Some("https://example.test/mcp".into()),
            ..Default::default()
        };
        assert_eq!(stdio.transport(), Some(BackendTransport::Stdio));
        assert_eq!(http.transport(), Some(BackendTransport::StreamableHttp));
    }
}
