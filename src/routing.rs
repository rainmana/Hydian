use std::{borrow::Cow, collections::BTreeMap, sync::Arc};

use anyhow::{Result, bail};
use rmcp::model::Tool;
use serde::Serialize;

use crate::backend::ManagedBackend;

#[derive(Clone)]
pub struct ToolRoute {
    pub exposed: Tool,
    pub backend: Arc<ManagedBackend>,
    pub original_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSummary {
    pub qualified_name: String,
    pub original_name: String,
    pub backend: String,
    pub description: Option<String>,
    pub available: bool,
    pub input_schema: serde_json::Value,
}

#[must_use]
pub fn sanitize_tool_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if matches!(
                character,
                'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/'
            ) {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "_".into()
    } else {
        sanitized
    }
}

pub fn build_catalog(
    backends: &BTreeMap<String, Arc<ManagedBackend>>,
    backend_tools: &BTreeMap<String, Vec<Tool>>,
    separator: &str,
) -> Result<BTreeMap<String, ToolRoute>> {
    let mut catalog = BTreeMap::new();
    for (backend_name, tools) in backend_tools {
        let backend = &backends[backend_name];
        let server_component = sanitize_tool_component(backend_name);
        for tool in tools {
            let original_name = tool.name.to_string();
            let tool_component = sanitize_tool_component(&original_name);
            let qualified_name = format!("{server_component}{separator}{tool_component}");
            if qualified_name.len() > 128 {
                bail!("qualified tool name `{qualified_name}` exceeds the MCP 128-character limit");
            }
            let mut exposed = tool.clone();
            exposed.name = Cow::Owned(qualified_name.clone());
            if let Some(previous) = catalog.insert(
                qualified_name.clone(),
                ToolRoute {
                    exposed,
                    backend: backend.clone(),
                    original_name,
                },
            ) {
                bail!(
                    "post-sanitization tool collision for `{qualified_name}` between backend `{}` and backend `{backend_name}`",
                    previous.backend.name()
                );
            }
        }
    }
    Ok(catalog)
}

#[must_use]
pub fn summarize(route: &ToolRoute, available: bool) -> ToolSummary {
    ToolSummary {
        qualified_name: route.exposed.name.to_string(),
        original_name: route.original_name.clone(),
        backend: route.backend.name().to_owned(),
        description: route.exposed.description.as_deref().map(ToOwned::to_owned),
        available,
        input_schema: serde_json::Value::Object((*route.exposed.input_schema).clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_tool_component;

    #[test]
    fn tool_name_sanitization_changes_only_forbidden_characters() {
        assert_eq!(
            sanitize_tool_component("read.file/path-v2"),
            "read.file/path-v2"
        );
        assert_eq!(sanitize_tool_component("read file:now"), "read_file_now");
    }
}
