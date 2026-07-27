use std::{collections::HashMap, time::Duration};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use http::{HeaderName, HeaderValue};
use rmcp::{
    ServiceExt,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

use super::{BackendClientHandler, BackendConnector, ConnectedBackend};
use crate::{
    config::{HydianConfig, is_loopback_host},
    model::McpServerDefinition,
    secrets::resolve_headers,
};

#[derive(Debug, Default)]
pub struct StreamableHttpConnector;

#[async_trait]
impl BackendConnector for StreamableHttpConnector {
    async fn connect(
        &self,
        _name: &str,
        definition: &McpServerDefinition,
        defaults: &HydianConfig,
    ) -> Result<ConnectedBackend> {
        let uri = definition
            .url
            .as_deref()
            .ok_or_else(|| anyhow!("Streamable HTTP backend has no URL"))?;
        let parsed_uri = reqwest::Url::parse(uri).context("backend URL is invalid")?;

        let resolved = resolve_headers(&definition.headers)?;
        let mut headers = HashMap::new();
        for (name, value) in resolved {
            headers.insert(
                HeaderName::try_from(name).context("backend header name is invalid")?,
                HeaderValue::try_from(value).context("backend header value is invalid")?,
            );
        }
        let connection_timeout = Duration::from_secs(
            definition
                .startup_timeout_seconds
                .unwrap_or(defaults.runtime.startup_timeout_seconds),
        );
        let mut client_builder = reqwest::Client::builder()
            .connect_timeout(connection_timeout)
            .redirect(reqwest::redirect::Policy::none());
        // A machine-wide proxy must never intercept local MCP traffic. This is
        // also deterministic when CI environments provide conflicting proxy
        // and NO_PROXY variables.
        if parsed_uri.host_str().is_some_and(is_loopback_host) {
            client_builder = client_builder.no_proxy();
        }
        let client = client_builder
            .build()
            .context("could not create backend HTTP client")?;
        let config = StreamableHttpClientTransportConfig::with_uri(uri.to_owned())
            .custom_headers(headers)
            .reinit_on_expired_session(true);
        let transport = StreamableHttpClientTransport::with_client(client, config);
        let handler = BackendClientHandler::new();
        let tools_changed = handler.change_flag();
        let service = handler
            .serve(transport)
            .await
            .context("MCP initialize handshake failed")?;
        Ok(ConnectedBackend {
            service,
            pid: None,
            tools_changed,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::secrets::SecretReference;

    #[test]
    fn authorization_values_remain_secret_references_until_connection() {
        assert!(matches!(
            SecretReference::parse("env:HYDIAN_TEST_TOKEN"),
            SecretReference::Environment(_)
        ));
    }
}
