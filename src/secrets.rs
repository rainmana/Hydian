use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use directories::BaseDirs;
use serde_json::Value;

const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretReference {
    Environment(String),
    File(PathBuf),
    Literal(String),
}

impl SecretReference {
    #[must_use]
    pub fn parse(value: &str) -> Self {
        if let Some(name) = value.strip_prefix("env:") {
            Self::Environment(name.to_owned())
        } else if let Some(path) = value.strip_prefix("file:") {
            Self::File(expand_tilde(Path::new(path)))
        } else {
            Self::Literal(value.to_owned())
        }
    }

    pub fn resolve(&self) -> Result<String> {
        match self {
            Self::Environment(name) => {
                env::var(name).with_context(|| format!("environment variable {name} is not set"))
            }
            Self::File(path) => {
                let value = fs::read_to_string(path)
                    .with_context(|| format!("could not read secret file {}", path.display()))?;
                Ok(value.trim_end_matches(['\r', '\n']).to_owned())
            }
            Self::Literal(value) => Ok(value.clone()),
        }
    }

    #[must_use]
    pub const fn is_literal(&self) -> bool {
        matches!(self, Self::Literal(_))
    }
}

pub fn resolve_headers(headers: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>> {
    headers
        .iter()
        .map(|(name, value)| {
            if name
                .chars()
                .any(|character| matches!(character, '\r' | '\n'))
            {
                bail!("header name contains a newline");
            }
            let resolved = SecretReference::parse(value).resolve()?;
            if resolved
                .chars()
                .any(|character| matches!(character, '\r' | '\n'))
            {
                bail!("header {name} resolves to a value containing a newline");
            }
            Ok((name.clone(), resolved))
        })
        .collect()
}

#[must_use]
pub fn redact_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    if is_secret_key(key) {
                        (key.clone(), Value::String(REDACTED.into()))
                    } else {
                        (key.clone(), redact_json(value))
                    }
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_json).collect()),
        other => other.clone(),
    }
}

#[must_use]
pub fn redact_text(input: &str, known_values: &[String]) -> String {
    let replaced = known_values
        .iter()
        .filter(|value| value.len() >= 4)
        .fold(input.to_owned(), |text, value| {
            text.replace(value, REDACTED)
        });
    if let Ok(value) = serde_json::from_str::<Value>(&replaced) {
        return serde_json::to_string(&redact_json(&value)).unwrap_or(replaced);
    }
    replaced
        .split_whitespace()
        .map(|token| {
            token
                .split_once('=')
                .filter(|(key, _)| is_secret_key(key))
                .map_or_else(|| token.to_owned(), |(key, _)| format!("{key}={REDACTED}"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    [
        "authorization",
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "private_key",
        "cookie",
    ]
    .iter()
    .any(|candidate| normalized.contains(candidate))
}

#[must_use]
pub fn literal_header_names(headers: &BTreeMap<String, String>) -> Vec<String> {
    headers
        .iter()
        .filter(|(name, value)| is_secret_key(name) && SecretReference::parse(value).is_literal())
        .map(|(name, _)| name.clone())
        .collect()
}

fn expand_tilde(path: &Path) -> PathBuf {
    let rendered = path.to_string_lossy();
    if rendered == "~" {
        return BaseDirs::new().map_or_else(
            || path.to_path_buf(),
            |directories| directories.home_dir().to_path_buf(),
        );
    }
    if let Some(suffix) = rendered
        .strip_prefix("~/")
        .or_else(|| rendered.strip_prefix(r"~\"))
    {
        return BaseDirs::new().map_or_else(
            || path.to_path_buf(),
            |directories| directories.home_dir().join(suffix),
        );
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{SecretReference, literal_header_names, redact_json, redact_text};

    #[test]
    fn classifies_supported_secret_references() {
        assert!(matches!(
            SecretReference::parse("env:TOKEN"),
            SecretReference::Environment(name) if name == "TOKEN"
        ));
        assert!(matches!(
            SecretReference::parse("literal"),
            SecretReference::Literal(value) if value == "literal"
        ));
    }

    #[test]
    fn redacts_nested_secret_fields() {
        let redacted = redact_json(&json!({
            "authorization": "Bearer abc",
            "nested": {"api_token": "abc", "safe": "visible"}
        }));
        assert_eq!(redacted["authorization"], "[REDACTED]");
        assert_eq!(redacted["nested"]["api_token"], "[REDACTED]");
        assert_eq!(redacted["nested"]["safe"], "visible");
    }

    #[test]
    fn reports_literal_secret_headers_without_values() {
        let headers = BTreeMap::from([
            ("Authorization".into(), "Bearer abc".into()),
            ("X-Token".into(), "env:API_TOKEN".into()),
        ]);
        assert_eq!(literal_header_names(&headers), vec!["Authorization"]);
    }

    #[test]
    fn redacts_known_values_from_text() {
        assert_eq!(
            redact_text("token abcdef leaked", &["abcdef".into()]),
            "token [REDACTED] leaked"
        );
    }
}
