pub mod stdio;
pub mod streamable_http;

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{MaybeSendFuture, NotificationContext, RequestContext},
};

use crate::runtime::Runtime;

#[derive(Clone)]
pub struct HydianServer {
    runtime: Arc<Runtime>,
}

impl HydianServer {
    #[must_use]
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
    }
}

impl ServerHandler for HydianServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_instructions("Hydian v0.1 multiplexes tools only. Backend sessions are shared.")
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        self.runtime
            .visible_tools()
            .await
            .map(ListToolsResult::with_all_items)
            .map_err(|error| McpError::internal_error(error.to_string(), None))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        match self.runtime.call_tool(request).await {
            Ok(result) => Ok(result),
            Err(error) if error.to_string().starts_with("unknown qualified tool") => {
                Err(McpError::invalid_params(error.to_string(), None))
            }
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(
                error.to_string(),
            )])),
        }
    }

    fn get_tool(&self, _name: &str) -> Option<Tool> {
        // The catalog is asynchronous. Input schemas are preserved and returned
        // by tools/list; downstream backends remain responsible for validation.
        None
    }

    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = ()> + MaybeSendFuture + '_ {
        let mut changes = self.runtime.subscribe_catalog_changes();
        let peer = context.peer;
        tokio::spawn(async move {
            loop {
                match changes.recv().await {
                    Ok(()) => {
                        if peer.notify_tool_list_changed().await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let _ = peer.notify_tool_list_changed().await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        std::future::ready(())
    }
}
