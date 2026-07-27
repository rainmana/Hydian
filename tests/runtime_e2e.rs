use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use hydian::{
    config::{HydianConfig, ProfileConfig, write_mcp_config},
    fixture::FixtureServer,
    frontend::streamable_http,
    model::{McpConfig, McpServerDefinition},
    paths::HydianPaths,
    runtime::{GatewayState, Runtime},
};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientInfo},
    transport::{
        StreamableHttpClientTransport, StreamableHttpServerConfig, StreamableHttpService,
        TokioChildProcess, streamable_http_client::StreamableHttpClientTransportConfig,
        streamable_http_server::session::local::LocalSessionManager,
    },
};
use serde_json::{Map, json};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

fn fixture_binary() -> String {
    env!("CARGO_BIN_EXE_hydian").to_owned()
}

fn stdio_server(label: &str) -> McpServerDefinition {
    McpServerDefinition {
        kind: Some("stdio".into()),
        command: Some(fixture_binary()),
        args: vec![
            "fixture".into(),
            "stdio".into(),
            "--label".into(),
            label.into(),
        ],
        startup_timeout_seconds: Some(5),
        request_timeout_seconds: Some(5),
        ..Default::default()
    }
}

fn stdio_server_with_behavior(label: &str, behavior: &str) -> McpServerDefinition {
    let mut definition = stdio_server(label);
    definition
        .args
        .extend(["--behavior".into(), behavior.into()]);
    definition
}

fn remote_server(url: String) -> McpServerDefinition {
    McpServerDefinition {
        kind: Some("streamable-http".into()),
        url: Some(url),
        startup_timeout_seconds: Some(5),
        request_timeout_seconds: Some(5),
        ..Default::default()
    }
}

async fn fixture_http() -> Result<(
    String,
    CancellationToken,
    tokio::task::JoinHandle<Result<()>>,
)> {
    let cancellation = CancellationToken::new();
    let handler = FixtureServer::new("remote".into(), hydian::cli::FixtureBehavior::Normal);
    let service: StreamableHttpService<FixtureServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(handler.clone()),
            Arc::default(),
            StreamableHttpServerConfig::default()
                .with_cancellation_token(cancellation.child_token()),
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let shutdown = cancellation.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled_owned().await })
            .await
            .context("fixture HTTP server failed")
    });
    Ok((format!("http://{address}/mcp"), cancellation, task))
}

fn test_paths(directory: &TempDir) -> Result<HydianPaths> {
    HydianPaths::resolve(Some(directory.path()), None, None)
}

#[tokio::test]
async fn aggregates_stdio_and_http_tools_routes_calls_and_survives_failure() -> Result<()> {
    let (remote_url, remote_shutdown, remote_task) = fixture_http().await?;
    let directory = TempDir::new()?;
    let paths = test_paths(&directory)?;
    paths.create_directories()?;
    let config = HydianConfig::default();
    let mcp = McpConfig {
        servers: BTreeMap::from([
            ("alpha".into(), stdio_server("alpha")),
            ("beta".into(), stdio_server("beta")),
            ("remote".into(), remote_server(remote_url)),
            (
                "missing".into(),
                McpServerDefinition {
                    kind: Some("stdio".into()),
                    command: Some("hydian-fixture-definitely-missing".into()),
                    startup_timeout_seconds: Some(1),
                    ..Default::default()
                },
            ),
        ]),
    };
    let runtime = Runtime::start(config, mcp, paths, None).await?;

    let tools = runtime.visible_tools().await?;
    let names = tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["alpha__echo", "beta__echo", "remote__echo"]);
    let alpha_schema = &tools
        .iter()
        .find(|tool| tool.name == "alpha__echo")
        .expect("alpha tool")
        .input_schema;
    assert!(alpha_schema.contains_key("properties"));

    for (qualified, expected_backend) in [
        ("alpha__echo", "alpha"),
        ("beta__echo", "beta"),
        ("remote__echo", "remote"),
    ] {
        let result = runtime
            .call_tool(
                CallToolRequestParams::new(qualified)
                    .with_arguments(Map::from_iter([("value".into(), json!("hello"))])),
            )
            .await?;
        assert_eq!(
            result.structured_content.as_ref().unwrap()["backend"],
            expected_backend
        );
    }

    let status = runtime.status().await;
    assert_eq!(status.state, GatewayState::Degraded);
    assert!(status.ready);
    assert!(status.unavailable_backends.contains(&"missing".into()));

    runtime.stop_backend("alpha").await?;
    let result = runtime
        .call_tool(CallToolRequestParams::new("remote__echo"))
        .await?;
    assert_eq!(result.structured_content.unwrap()["backend"], "remote");

    runtime.shutdown().await;
    remote_shutdown.cancel();
    remote_task.await??;
    Ok(())
}

#[tokio::test]
async fn http_frontend_lists_and_calls_the_shared_catalog() -> Result<()> {
    let directory = TempDir::new()?;
    let paths = test_paths(&directory)?;
    paths.create_directories()?;
    let runtime = Runtime::start(
        HydianConfig::default(),
        McpConfig {
            servers: BTreeMap::from([("alpha".into(), stdio_server("alpha"))]),
        },
        paths,
        None,
    )
    .await?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let frontend = streamable_http::start_with_listener(runtime.clone(), listener)?;
    let url = format!("http://{}/mcp", frontend.address);
    let http = reqwest::Client::builder().no_proxy().build()?;
    assert!(
        http.get(format!("http://{}/healthz", frontend.address))
            .send()
            .await?
            .status()
            .is_success()
    );
    let ready: serde_json::Value = http
        .get(format!("http://{}/readyz", frontend.address))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(ready["ready"], true);
    let forbidden = http
        .post(&url)
        .header("Origin", "https://attacker.example")
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
        .send()
        .await?;
    assert_eq!(forbidden.status(), reqwest::StatusCode::FORBIDDEN);
    let control = http
        .post(format!(
            "http://{}/control/servers/alpha/restart",
            frontend.address
        ))
        .send()
        .await?;
    assert!(control.status().is_success());
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::builder().no_proxy().build()?,
        StreamableHttpClientTransportConfig::with_uri(url),
    );
    let client = ClientInfo::default().serve(transport).await?;

    let tools = client.list_all_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "alpha__echo");
    let result = client
        .call_tool(
            CallToolRequestParams::new("alpha__echo")
                .with_arguments(Map::from_iter([("value".into(), json!(42))])),
        )
        .await?;
    assert_eq!(result.structured_content.unwrap()["value"], 42);

    client.cancel().await?;
    frontend.shutdown().await?;
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn stdio_frontend_never_prefixes_protocol_stdout_with_human_output() -> Result<()> {
    let directory = TempDir::new()?;
    let paths = test_paths(&directory)?;
    paths.create_directories()?;
    HydianConfig::default().write(&paths.config, &paths.backups)?;
    write_mcp_config(
        &McpConfig {
            servers: BTreeMap::from([("inner".into(), stdio_server("inner"))]),
        },
        &paths.mcp_config,
        &paths.backups,
    )?;

    let mut command = tokio::process::Command::new(fixture_binary());
    command.args(["--home", &paths.home.to_string_lossy(), "stdio"]);
    let transport = TokioChildProcess::new(command)?;
    let client = tokio::time::timeout(
        Duration::from_secs(10),
        ClientInfo::default().serve(transport),
    )
    .await??;
    let tools = client.list_all_tools().await?;
    assert_eq!(tools[0].name, "inner__echo");
    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn profiles_filter_the_live_catalog_without_rewriting_mcp_configuration() -> Result<()> {
    let directory = TempDir::new()?;
    let paths = test_paths(&directory)?;
    paths.create_directories()?;
    let mut config = HydianConfig::default();
    config.profiles.insert(
        "alpha-only".into(),
        ProfileConfig {
            servers: vec!["alpha".into()],
        },
    );
    let runtime = Runtime::start(
        config,
        McpConfig {
            servers: BTreeMap::from([
                ("alpha".into(), stdio_server("alpha")),
                ("beta".into(), stdio_server("beta")),
            ]),
        },
        paths,
        None,
    )
    .await?;
    runtime.activate_profile("alpha-only").await?;
    let tools = runtime.visible_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "alpha__echo");
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn backend_list_changed_refreshes_the_qualified_catalog() -> Result<()> {
    let directory = TempDir::new()?;
    let paths = test_paths(&directory)?;
    paths.create_directories()?;
    let runtime = Runtime::start(
        HydianConfig::default(),
        McpConfig {
            servers: BTreeMap::from([(
                "dynamic".into(),
                stdio_server_with_behavior("dynamic", "list-changed"),
            )]),
        },
        paths,
        None,
    )
    .await?;
    assert_eq!(runtime.visible_tools().await?.len(), 1);
    runtime
        .call_tool(CallToolRequestParams::new("dynamic__echo"))
        .await?;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let names = runtime
        .visible_tools()
        .await?
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["dynamic__added", "dynamic__echo"]);
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn ambiguous_tool_timeouts_are_not_retried() -> Result<()> {
    let directory = TempDir::new()?;
    let paths = test_paths(&directory)?;
    paths.create_directories()?;
    let mut slow = stdio_server_with_behavior("slow", "slow");
    slow.request_timeout_seconds = Some(1);
    let runtime = Runtime::start(
        HydianConfig::default(),
        McpConfig {
            servers: BTreeMap::from([("slow".into(), slow)]),
        },
        paths,
        None,
    )
    .await?;
    let error = runtime
        .call_tool(CallToolRequestParams::new("slow__echo"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("may have executed"));
    runtime.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn post_sanitization_tool_collisions_are_rejected() -> Result<()> {
    let directory = TempDir::new()?;
    let paths = test_paths(&directory)?;
    paths.create_directories()?;
    let error = Runtime::start(
        HydianConfig::default(),
        McpConfig {
            servers: BTreeMap::from([
                ("same name".into(), stdio_server("one")),
                ("same:name".into(), stdio_server("two")),
            ]),
        },
        paths,
        None,
    )
    .await
    .err()
    .expect("sanitized names should collide");
    assert!(
        error
            .to_string()
            .contains("post-sanitization tool collision")
    );
    Ok(())
}
