use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use rmcp::transport::{
    StreamableHttpServerConfig, StreamableHttpService,
    streamable_http_server::session::local::LocalSessionManager,
};
use serde_json::json;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use super::HydianServer;
use crate::{runtime::Runtime, security::allowed_origins};

pub struct HttpFrontend {
    pub address: SocketAddr,
    cancellation: CancellationToken,
    task: JoinHandle<Result<()>>,
}

impl HttpFrontend {
    pub async fn shutdown(self) -> Result<()> {
        self.cancellation.cancel();
        self.task
            .await
            .context("HTTP frontend task did not complete")??;
        Ok(())
    }
}

pub async fn start(runtime: Arc<Runtime>) -> Result<HttpFrontend> {
    let config = runtime.config();
    let listener = TcpListener::bind((config.listener.host.as_str(), config.listener.port))
        .await
        .with_context(|| {
            format!(
                "could not bind HTTP listener {}:{}",
                config.listener.host, config.listener.port
            )
        })?;
    start_with_listener(runtime, listener)
}

pub fn start_with_listener(runtime: Arc<Runtime>, listener: TcpListener) -> Result<HttpFrontend> {
    let address = listener
        .local_addr()
        .context("could not read HTTP listener address")?;
    let cancellation = CancellationToken::new();
    let origins = if runtime.config().security.validate_origin {
        allowed_origins(runtime.config())
    } else {
        Vec::new()
    };
    let server_config = StreamableHttpServerConfig::default()
        .disable_allowed_hosts()
        .with_allowed_origins(origins)
        .with_cancellation_token(cancellation.child_token());
    let handler_runtime = runtime.clone();
    let service: StreamableHttpService<HydianServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(HydianServer::new(handler_runtime.clone())),
            Arc::default(),
            server_config,
        );

    let mcp_path = runtime.config().listener.path.clone();
    let router = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(readiness))
        .route("/status", get(status))
        .route(
            "/control/servers/{name}/{action}",
            axum::routing::post(control_server),
        )
        .route(
            "/control/profiles/{name}",
            axum::routing::post(control_profile),
        )
        .nest_service(&mcp_path, service)
        .with_state(runtime);
    let graceful = cancellation.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { graceful.cancelled_owned().await })
            .await
            .context("HTTP frontend stopped with an error")
    });

    Ok(HttpFrontend {
        address,
        cancellation,
        task,
    })
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"alive": true}))
}

async fn readiness(State(runtime): State<Arc<Runtime>>) -> Response {
    let status = runtime.status().await;
    let code = if status.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (code, Json(status)).into_response()
}

async fn status(State(runtime): State<Arc<Runtime>>) -> Json<crate::runtime::RuntimeStatus> {
    Json(runtime.status().await)
}

async fn control_server(
    State(runtime): State<Arc<Runtime>>,
    Path((name, action)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    if !crate::security::validate_origin(&headers, runtime.config()).allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"ok": false, "error": "Origin is not allowed"})),
        )
            .into_response();
    }
    let result = match action.as_str() {
        "start" => runtime.start_backend(&name).await,
        "stop" => runtime.stop_backend(&name).await,
        "restart" => runtime.restart_backend(&name).await,
        _ => Err(anyhow::anyhow!(
            "unknown backend action `{action}`; choose start, stop, or restart"
        )),
    };
    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "server": name, "action": action})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        )
            .into_response(),
    }
}

async fn control_profile(
    State(runtime): State<Arc<Runtime>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !crate::security::validate_origin(&headers, runtime.config()).allowed {
        return StatusCode::FORBIDDEN.into_response();
    }
    match runtime.activate_profile(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "profile": name})),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": error.to_string()})),
        )
            .into_response(),
    }
}
