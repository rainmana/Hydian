use http::{HeaderMap, HeaderValue, header::ORIGIN};
use serde::Serialize;

use crate::config::HydianConfig;

#[derive(Debug, Clone, Serialize)]
pub struct OriginDecision {
    pub allowed: bool,
    pub reason: String,
}

#[must_use]
pub fn allowed_origins(config: &HydianConfig) -> Vec<String> {
    if !config.security.allowed_origins.is_empty() {
        return config.security.allowed_origins.clone();
    }

    let port = config.listener.port;
    vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("http://[::1]:{port}"),
    ]
}

#[must_use]
pub fn validate_origin(headers: &HeaderMap, config: &HydianConfig) -> OriginDecision {
    if !config.security.validate_origin {
        return OriginDecision {
            allowed: true,
            reason: "Origin validation is explicitly disabled".into(),
        };
    }
    let Some(origin) = headers.get(ORIGIN) else {
        return OriginDecision {
            allowed: true,
            reason: "request has no Origin header".into(),
        };
    };
    let Ok(origin) = origin.to_str() else {
        return OriginDecision {
            allowed: false,
            reason: "Origin header is not valid text".into(),
        };
    };
    let allowed = allowed_origins(config)
        .iter()
        .any(|candidate| candidate == origin);
    OriginDecision {
        allowed,
        reason: if allowed {
            format!("Origin `{origin}` is explicitly allowed")
        } else {
            format!(
                "Origin `{origin}` is not in security.allowed_origins; browser requests are rejected to prevent DNS rebinding"
            )
        },
    }
}

#[must_use]
pub fn origin_header(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(ORIGIN, value);
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::{origin_header, validate_origin};
    use crate::config::HydianConfig;

    #[test]
    fn default_origin_policy_accepts_local_and_rejects_foreign_origins() {
        let config = HydianConfig::default();
        assert!(validate_origin(&origin_header("http://127.0.0.1:7337"), &config).allowed);
        assert!(!validate_origin(&origin_header("https://attacker.example"), &config).allowed);
    }

    #[test]
    fn requests_without_an_origin_are_allowed_for_native_clients() {
        let config = HydianConfig::default();
        assert!(validate_origin(&http::HeaderMap::new(), &config).allowed);
    }
}
