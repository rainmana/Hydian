use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    transport::{
        StreamableHttpServerConfig, StreamableHttpService, stdio,
        streamable_http_server::session::local::LocalSessionManager,
    },
};
use serde_json::{Map, Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::cli::{FixtureArgs, FixtureBehavior, FixtureTransport};

#[derive(Clone)]
pub struct FixtureServer {
    label: String,
    behavior: FixtureBehavior,
    calls: Arc<AtomicUsize>,
}

impl FixtureServer {
    #[must_use]
    pub fn new(label: String, behavior: FixtureBehavior) -> Self {
        Self {
            label,
            behavior,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn echo_tool(&self) -> Tool {
        Tool::new(
            "echo",
            format!("Echo a value from the {} fixture", self.label),
            Arc::new(Map::from_iter([
                ("type".into(), Value::String("object".into())),
                (
                    "properties".into(),
                    json!({"value": {"description": "Value to echo"}}),
                ),
            ])),
        )
    }

    fn changed_tool() -> Tool {
        Tool::new(
            "added",
            "Tool added after the first call",
            Arc::new(Map::from_iter([(
                "type".into(),
                Value::String("object".into()),
            )])),
        )
    }
}

impl ServerHandler for FixtureServer {
    fn get_info(&self) -> ServerInfo {
        let capabilities = if self.behavior == FixtureBehavior::ListChanged {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        };
        ServerInfo::new(capabilities)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut tools = vec![self.echo_tool()];
        if self.behavior == FixtureBehavior::ListChanged && self.calls.load(Ordering::SeqCst) > 0 {
            tools.push(Self::changed_tool());
        }
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if request.name != "echo" && request.name != "added" {
            return Err(McpError::invalid_params("unknown fixture tool", None));
        }
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.behavior == FixtureBehavior::ListChanged && call == 1 {
            let peer = context.peer;
            tokio::spawn(async move {
                let _ = peer.notify_tool_list_changed().await;
            });
        }
        if self.behavior == FixtureBehavior::ExitAfterOne && call == 1 {
            tokio::spawn(async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                std::process::exit(0);
            });
        }
        if self.behavior == FixtureBehavior::Slow {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let value = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("value"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(CallToolResult::structured(json!({
            "backend": self.label,
            "value": value,
            "call": call,
        })))
    }
}

pub async fn run(args: FixtureArgs) -> Result<()> {
    if args.behavior == FixtureBehavior::FailStartup {
        bail!("fixture intentionally failed during startup");
    }
    let handler = FixtureServer::new(args.label, args.behavior);
    match args.transport {
        FixtureTransport::Stdio => {
            handler
                .serve(stdio())
                .await
                .context("fixture stdio initialization failed")?
                .waiting()
                .await
                .context("fixture stdio task failed")?;
        }
        FixtureTransport::Http => run_http(handler, &args.listen).await?,
    }
    Ok(())
}

async fn run_http(handler: FixtureServer, listen: &str) -> Result<()> {
    let listener = TcpListener::bind(listen)
        .await
        .with_context(|| format!("could not bind fixture listener {listen}"))?;
    let address = listener.local_addr()?;
    eprintln!(
        "{}",
        json!({"fixture_url": format!("http://{address}/mcp")})
    );
    let cancellation = CancellationToken::new();
    let service: StreamableHttpService<FixtureServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(handler.clone()),
            Arc::default(),
            StreamableHttpServerConfig::default()
                .with_cancellation_token(cancellation.child_token()),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("fixture HTTP server failed")
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
