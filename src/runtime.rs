use std::{collections::BTreeMap, fs, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use rmcp::model::{CallToolRequestParams, CallToolResult, Tool};
use serde::Serialize;
use tempfile::NamedTempFile;
use tokio::sync::{RwLock, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    backend::{
        BackendSnapshot, BackendState, ManagedBackend, stdio::StdioConnector,
        streamable_http::StreamableHttpConnector,
    },
    config::HydianConfig,
    model::{BackendTransport, McpConfig},
    paths::HydianPaths,
    profiles::visible_server_names,
    routing::{ToolRoute, ToolSummary, build_catalog, summarize},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GatewayState {
    Ready,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    pub schema_version: u32,
    pub generated_at: DateTime<Utc>,
    pub state: GatewayState,
    pub ready: bool,
    pub degraded: bool,
    pub endpoint: String,
    pub active_profile: String,
    pub ready_backends: Vec<String>,
    pub unavailable_backends: Vec<String>,
    pub tool_count: usize,
    pub tools: Vec<ToolSummary>,
    pub backends: Vec<BackendSnapshot>,
    pub active_exposure_provider: Option<String>,
}

pub struct Runtime {
    config: HydianConfig,
    paths: HydianPaths,
    backends: BTreeMap<String, Arc<ManagedBackend>>,
    catalog: RwLock<BTreeMap<String, ToolRoute>>,
    active_profile: RwLock<String>,
    catalog_changed: broadcast::Sender<()>,
    cancellation: CancellationToken,
}

impl Runtime {
    pub async fn start(
        config: HydianConfig,
        mcp: McpConfig,
        paths: HydianPaths,
        profile_override: Option<&str>,
    ) -> Result<Arc<Self>> {
        let active_profile = profile_override
            .unwrap_or(&config.runtime.active_profile)
            .to_owned();
        if !config.profiles.contains_key(&active_profile) {
            bail!("profile `{active_profile}` is not defined");
        }

        let mut backends = BTreeMap::new();
        for (name, definition) in mcp.servers {
            if !definition.enabled {
                continue;
            }
            let connector: Arc<dyn crate::backend::BackendConnector> = match definition.transport()
            {
                Some(BackendTransport::Stdio) => Arc::new(StdioConnector),
                Some(BackendTransport::StreamableHttp) => Arc::new(StreamableHttpConnector),
                None => {
                    tracing::error!(backend = %name, "backend has no recognized transport");
                    continue;
                }
            };
            backends.insert(
                name.clone(),
                Arc::new(ManagedBackend::new(
                    name,
                    definition,
                    config.clone(),
                    connector,
                )),
            );
        }

        let (catalog_changed, _) = broadcast::channel(32);
        let runtime = Arc::new(Self {
            config,
            paths,
            backends,
            catalog: RwLock::new(BTreeMap::new()),
            active_profile: RwLock::new(active_profile),
            catalog_changed,
            cancellation: CancellationToken::new(),
        });
        runtime.start_backends().await;
        runtime.refresh_catalog().await?;
        runtime.spawn_catalog_monitor();
        runtime.write_status().await?;
        Ok(runtime)
    }

    async fn start_backends(&self) {
        for backend in self.backends.values() {
            if let Err(error) = backend.start().await {
                tracing::error!(
                    backend = backend.name(),
                    error = %error,
                    "backend is unavailable; healthy backends remain active"
                );
            }
        }
    }

    pub async fn refresh_catalog(&self) -> Result<()> {
        let mut backend_tools = BTreeMap::new();
        for (name, backend) in &self.backends {
            let snapshot = backend.snapshot().await;
            if matches!(snapshot.state, BackendState::Ready | BackendState::Degraded) {
                backend_tools.insert(name.clone(), backend.tools().await);
            }
        }
        let catalog = build_catalog(
            &self.backends,
            &backend_tools,
            &self.config.naming.separator,
        )?;
        let mut current = self.catalog.write().await;
        let previous = current
            .values()
            .map(|route| route.exposed.clone())
            .collect::<Vec<_>>();
        let next = catalog
            .values()
            .map(|route| route.exposed.clone())
            .collect::<Vec<_>>();
        *current = catalog;
        drop(current);
        if previous != next {
            let _ = self.catalog_changed.send(());
        }
        Ok(())
    }

    fn spawn_catalog_monitor(self: &Arc<Self>) {
        let runtime = Arc::downgrade(self);
        let cancellation = self.cancellation.child_token();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                tokio::select! {
                    () = cancellation.cancelled() => break,
                    _ = interval.tick() => {
                        let Some(runtime) = runtime.upgrade() else {
                            break;
                        };
                        if let Err(error) = runtime.apply_tool_changes().await {
                            tracing::warn!(%error, "could not apply backend tool catalog change");
                        }
                    }
                }
            }
        });
    }

    pub fn subscribe_catalog_changes(&self) -> broadcast::Receiver<()> {
        self.catalog_changed.subscribe()
    }

    pub async fn visible_tools(&self) -> Result<Vec<Tool>> {
        self.apply_tool_changes().await?;
        let visible = self.visible_backend_names().await?;
        Ok(self
            .catalog
            .read()
            .await
            .values()
            .filter(|route| visible.contains(route.backend.name()))
            .map(|route| route.exposed.clone())
            .collect())
    }

    pub async fn tool_summaries(&self) -> Result<Vec<ToolSummary>> {
        self.apply_tool_changes().await?;
        let visible = self.visible_backend_names().await?;
        let catalog = self.catalog.read().await;
        let mut summaries = Vec::new();
        for route in catalog.values() {
            if visible.contains(route.backend.name()) {
                let snapshot = route.backend.snapshot().await;
                summaries.push(summarize(
                    route,
                    matches!(snapshot.state, BackendState::Ready),
                ));
            }
        }
        Ok(summaries)
    }

    async fn apply_tool_changes(&self) -> Result<()> {
        let mut changed = false;
        for backend in self.backends.values() {
            if backend.take_tools_changed().await {
                match backend.refresh_tools().await {
                    Ok(_) => changed = true,
                    Err(error) => {
                        tracing::warn!(
                            backend = backend.name(),
                            %error,
                            "could not refresh catalog after tools/list_changed"
                        );
                    }
                }
            }
        }
        if changed {
            self.refresh_catalog().await?;
        }
        Ok(())
    }

    pub async fn call_tool(&self, mut request: CallToolRequestParams) -> Result<CallToolResult> {
        let visible = self.visible_backend_names().await?;
        let route = self
            .catalog
            .read()
            .await
            .get(request.name.as_ref())
            .cloned()
            .ok_or_else(|| anyhow!("unknown qualified tool `{}`", request.name))?;
        if !visible.contains(route.backend.name()) {
            bail!(
                "tool `{}` is hidden by active profile `{}`",
                request.name,
                self.active_profile.read().await
            );
        }
        request.name = route.original_name.into();
        route.backend.call(request).await
    }

    pub async fn activate_profile(&self, name: &str) -> Result<()> {
        if !self.config.profiles.contains_key(name) {
            bail!("profile `{name}` is not defined");
        }
        *self.active_profile.write().await = name.to_owned();
        self.write_status().await
    }

    async fn visible_backend_names(&self) -> Result<std::collections::BTreeSet<String>> {
        let profile = self.active_profile.read().await.clone();
        let configured = self
            .backends
            .iter()
            .map(|(name, backend)| {
                (
                    name.clone(),
                    crate::model::McpServerDefinition {
                        kind: Some(match backend.transport() {
                            BackendTransport::Stdio => "stdio".into(),
                            BackendTransport::StreamableHttp => "streamable-http".into(),
                        }),
                        enabled: true,
                        ..Default::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        visible_server_names(
            &self.config,
            &McpConfig {
                servers: configured,
            },
            Some(&profile),
        )
    }

    pub async fn status(&self) -> RuntimeStatus {
        let mut snapshots = Vec::new();
        for backend in self.backends.values() {
            snapshots.push(backend.snapshot().await);
        }
        let ready_backends = snapshots
            .iter()
            .filter(|snapshot| snapshot.state == BackendState::Ready)
            .map(|snapshot| snapshot.name.clone())
            .collect::<Vec<_>>();
        let unavailable_backends = snapshots
            .iter()
            .filter(|snapshot| snapshot.state != BackendState::Ready)
            .map(|snapshot| snapshot.name.clone())
            .collect::<Vec<_>>();
        let tools = self.tool_summaries().await.unwrap_or_default();
        let tool_count = tools.len();
        let state = if unavailable_backends.is_empty() {
            GatewayState::Ready
        } else {
            GatewayState::Degraded
        };
        RuntimeStatus {
            schema_version: 1,
            generated_at: Utc::now(),
            state,
            ready: state != GatewayState::Failed,
            degraded: state == GatewayState::Degraded,
            endpoint: self.config.endpoint(),
            active_profile: self.active_profile.read().await.clone(),
            ready_backends,
            unavailable_backends,
            tool_count,
            tools,
            backends: snapshots,
            active_exposure_provider: self.config.exposure.active_provider.clone(),
        }
    }

    pub async fn write_status(&self) -> Result<()> {
        let status = self.status().await;
        write_status_file(&self.paths.status, &status)
    }

    pub async fn stop_backend(&self, name: &str) -> Result<()> {
        let backend = self
            .backends
            .get(name)
            .ok_or_else(|| anyhow!("backend `{name}` is not configured"))?;
        backend.stop().await?;
        self.refresh_catalog().await?;
        self.write_status().await
    }

    pub async fn start_backend(&self, name: &str) -> Result<()> {
        let backend = self
            .backends
            .get(name)
            .ok_or_else(|| anyhow!("backend `{name}` is not configured"))?;
        backend.start().await?;
        self.refresh_catalog().await?;
        self.write_status().await
    }

    pub async fn restart_backend(&self, name: &str) -> Result<()> {
        let backend = self
            .backends
            .get(name)
            .ok_or_else(|| anyhow!("backend `{name}` is not configured"))?;
        backend.restart().await?;
        self.refresh_catalog().await?;
        self.write_status().await
    }

    pub async fn shutdown(&self) {
        self.cancellation.cancel();
        for backend in self.backends.values() {
            if let Err(error) = backend.stop().await {
                tracing::warn!(backend = backend.name(), %error, "backend shutdown failed");
            }
        }
        if let Err(error) = self.write_status().await {
            tracing::warn!(%error, "could not write final runtime status");
        }
    }

    #[must_use]
    pub fn config(&self) -> &HydianConfig {
        &self.config
    }
}

fn write_status_file(path: &Path, status: &RuntimeStatus) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("status path has no parent directory"))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("could not create runtime directory {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(status).context("could not serialize runtime status")?;
    let mut temporary =
        NamedTempFile::new_in(parent).context("could not create temporary status file")?;
    std::io::Write::write_all(&mut temporary, &bytes)
        .context("could not write temporary status file")?;
    temporary
        .as_file()
        .sync_all()
        .context("could not flush temporary status file")?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("could not replace runtime status {}", path.display()))?;
    Ok(())
}
