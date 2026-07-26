pub mod stdio;
pub mod streamable_http;

use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use rmcp::{
    ClientHandler, RoleClient,
    model::{CallToolRequestParams, CallToolResult, Tool},
    service::{MaybeSendFuture, NotificationContext, RunningService},
};
use serde::Serialize;
use tokio::sync::{Mutex, RwLock, Semaphore};

use crate::{
    config::{HydianConfig, RestartConfig},
    model::{BackendTransport, McpServerDefinition},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendState {
    Starting,
    Ready,
    Degraded,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendSnapshot {
    pub name: String,
    pub transport: BackendTransport,
    pub state: BackendState,
    pub tool_count: usize,
    pub pid: Option<u32>,
    pub restart_count: u64,
    pub uptime_seconds: Option<u64>,
    pub last_error: Option<String>,
}

pub struct ConnectedBackend {
    pub service: RunningService<RoleClient, BackendClientHandler>,
    pub pid: Option<u32>,
    pub tools_changed: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Default)]
pub struct BackendClientHandler {
    tools_changed: Arc<std::sync::atomic::AtomicBool>,
}

impl BackendClientHandler {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn change_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.tools_changed.clone()
    }
}

impl ClientHandler for BackendClientHandler {
    fn on_tool_list_changed(
        &self,
        _context: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + MaybeSendFuture + '_ {
        self.tools_changed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        std::future::ready(())
    }
}

#[async_trait]
pub trait BackendConnector: Send + Sync {
    async fn connect(
        &self,
        name: &str,
        definition: &McpServerDefinition,
        defaults: &HydianConfig,
    ) -> Result<ConnectedBackend>;
}

struct BackendMutable {
    service: Option<RunningService<RoleClient, BackendClientHandler>>,
    tools_changed: Option<Arc<std::sync::atomic::AtomicBool>>,
    state: BackendState,
    pid: Option<u32>,
    restart_count: u64,
    started_at: Option<Instant>,
    last_error: Option<String>,
    tool_count: usize,
    restart_times: VecDeque<Instant>,
}

pub struct ManagedBackend {
    name: String,
    transport: BackendTransport,
    definition: McpServerDefinition,
    defaults: HydianConfig,
    connector: Arc<dyn BackendConnector>,
    mutable: Mutex<BackendMutable>,
    connect_lock: Mutex<()>,
    semaphore: Semaphore,
    tools: RwLock<Vec<Tool>>,
}

impl ManagedBackend {
    #[must_use]
    pub fn new(
        name: String,
        definition: McpServerDefinition,
        defaults: HydianConfig,
        connector: Arc<dyn BackendConnector>,
    ) -> Self {
        let transport = definition
            .transport()
            .expect("validated backend definitions have a transport");
        Self {
            name,
            transport,
            definition,
            semaphore: Semaphore::new(defaults.servers.defaults.max_concurrent_calls),
            defaults,
            connector,
            mutable: Mutex::new(BackendMutable {
                service: None,
                tools_changed: None,
                state: BackendState::Stopped,
                pid: None,
                restart_count: 0,
                started_at: None,
                last_error: None,
                tool_count: 0,
                restart_times: VecDeque::new(),
            }),
            connect_lock: Mutex::new(()),
            tools: RwLock::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn transport(&self) -> BackendTransport {
        self.transport
    }

    pub async fn start(&self) -> Result<()> {
        let _connect_guard = self.connect_lock.lock().await;
        if self.is_connected().await {
            return Ok(());
        }
        self.connect_once().await
    }

    async fn connect_once(&self) -> Result<()> {
        {
            let mut mutable = self.mutable.lock().await;
            mutable.state = BackendState::Starting;
            mutable.last_error = None;
        }

        let timeout_seconds = self
            .definition
            .startup_timeout_seconds
            .unwrap_or(self.defaults.runtime.startup_timeout_seconds);
        let result = tokio::time::timeout(
            Duration::from_secs(timeout_seconds),
            self.connector
                .connect(&self.name, &self.definition, &self.defaults),
        )
        .await
        .map_err(|_| anyhow!("startup timed out after {timeout_seconds} seconds"))
        .and_then(std::convert::identity);

        match result {
            Ok(connected) => {
                let peer = connected.service.peer().clone();
                let request_timeout = self.request_timeout();
                let tools = tokio::time::timeout(request_timeout, peer.list_all_tools())
                    .await
                    .map_err(|_| anyhow!("tools/list timed out after {request_timeout:?}"))?
                    .context("backend rejected tools/list during startup")?;
                let count = tools.len();
                *self.tools.write().await = tools;
                let mut mutable = self.mutable.lock().await;
                mutable.service = Some(connected.service);
                mutable.tools_changed = Some(connected.tools_changed);
                mutable.pid = connected.pid;
                mutable.state = BackendState::Ready;
                mutable.started_at = Some(Instant::now());
                mutable.tool_count = count;
                mutable.last_error = None;
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                let mut mutable = self.mutable.lock().await;
                mutable.state = BackendState::Failed;
                mutable.pid = None;
                mutable.started_at = None;
                mutable.tool_count = 0;
                mutable.last_error = Some(message.clone());
                Err(anyhow!(message))
            }
        }
    }

    pub async fn ensure_connected(&self) -> Result<()> {
        if self.is_connected().await {
            return Ok(());
        }
        let _connect_guard = self.connect_lock.lock().await;
        if self.is_connected().await {
            return Ok(());
        }
        self.apply_restart_policy().await?;
        self.connect_once().await
    }

    async fn apply_restart_policy(&self) -> Result<()> {
        let restart = &self.defaults.restart;
        if !restart.enabled {
            return Err(anyhow!("backend restart is disabled"));
        }
        let mut mutable = self.mutable.lock().await;
        let now = Instant::now();
        while mutable
            .restart_times
            .front()
            .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
        {
            mutable.restart_times.pop_front();
        }
        if mutable.restart_times.len()
            >= usize::try_from(restart.maximum_restarts_per_minute).unwrap_or(usize::MAX)
        {
            mutable.state = BackendState::Failed;
            return Err(anyhow!(
                "restart limit reached: {} attempts in one minute",
                restart.maximum_restarts_per_minute
            ));
        }
        let delay = restart_delay(restart, mutable.restart_count);
        mutable.restart_count += 1;
        mutable.restart_times.push_back(now);
        drop(mutable);
        tokio::time::sleep(delay).await;
        Ok(())
    }

    async fn is_connected(&self) -> bool {
        self.mutable
            .lock()
            .await
            .service
            .as_ref()
            .is_some_and(|service| !service.is_closed())
    }

    pub async fn tools(&self) -> Vec<Tool> {
        self.tools.read().await.clone()
    }

    pub async fn take_tools_changed(&self) -> bool {
        self.mutable
            .lock()
            .await
            .tools_changed
            .as_ref()
            .is_some_and(|flag| flag.swap(false, std::sync::atomic::Ordering::SeqCst))
    }

    pub async fn refresh_tools(&self) -> Result<Vec<Tool>> {
        self.ensure_connected().await?;
        let peer = {
            let mutable = self.mutable.lock().await;
            mutable
                .service
                .as_ref()
                .map(|service| service.peer().clone())
                .ok_or_else(|| anyhow!("backend session is not available"))?
        };
        let timeout = self.request_timeout();
        let tools = tokio::time::timeout(timeout, peer.list_all_tools())
            .await
            .map_err(|_| anyhow!("tools/list timed out after {timeout:?}"))?
            .context("backend tools/list failed")?;
        let count = tools.len();
        *self.tools.write().await = tools.clone();
        let mut mutable = self.mutable.lock().await;
        mutable.tool_count = count;
        mutable.state = BackendState::Ready;
        Ok(tools)
    }

    pub async fn call(&self, request: CallToolRequestParams) -> Result<CallToolResult> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .context("backend concurrency limiter was closed")?;
        self.ensure_connected().await?;
        let peer = {
            let mutable = self.mutable.lock().await;
            mutable
                .service
                .as_ref()
                .map(|service| service.peer().clone())
                .ok_or_else(|| anyhow!("backend session is not available"))?
        };
        let timeout = self.request_timeout();
        match tokio::time::timeout(timeout, peer.call_tool(request)).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => {
                self.mark_degraded(format!(
                    "tool call transport failed; execution may be ambiguous and was not retried: {error}"
                ))
                .await;
                Err(anyhow!(
                    "backend transport failed; the call may have executed and Hydian did not retry it: {error}"
                ))
            }
            Err(_) => {
                self.mark_degraded(format!(
                    "tool call timed out after {timeout:?}; execution may be ambiguous"
                ))
                .await;
                Err(anyhow!(
                    "backend timed out after {timeout:?}; the call may have executed and was not retried"
                ))
            }
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let mut service = {
            let mut mutable = self.mutable.lock().await;
            mutable.state = BackendState::Stopped;
            mutable.pid = None;
            mutable.started_at = None;
            mutable.service.take()
        };
        if let Some(service) = service.as_mut() {
            let grace = Duration::from_secs(self.defaults.runtime.shutdown_grace_seconds);
            service
                .close_with_timeout(grace)
                .await
                .context("backend shutdown task failed")?;
        }
        Ok(())
    }

    pub async fn restart(&self) -> Result<()> {
        self.stop().await?;
        self.ensure_connected().await
    }

    pub async fn snapshot(&self) -> BackendSnapshot {
        let mutable = self.mutable.lock().await;
        let state = if mutable
            .service
            .as_ref()
            .is_some_and(rmcp::service::RunningService::is_closed)
            && mutable.state == BackendState::Ready
        {
            BackendState::Degraded
        } else {
            mutable.state
        };
        BackendSnapshot {
            name: self.name.clone(),
            transport: self.transport,
            state,
            tool_count: mutable.tool_count,
            pid: mutable.pid,
            restart_count: mutable.restart_count,
            uptime_seconds: mutable.started_at.map(|time| time.elapsed().as_secs()),
            last_error: mutable.last_error.clone(),
        }
    }

    async fn mark_degraded(&self, error: String) {
        let mut mutable = self.mutable.lock().await;
        mutable.state = BackendState::Degraded;
        mutable.last_error = Some(error);
    }

    fn request_timeout(&self) -> Duration {
        Duration::from_secs(
            self.definition
                .request_timeout_seconds
                .unwrap_or(self.defaults.runtime.request_timeout_seconds),
        )
    }
}

#[must_use]
pub fn restart_delay(config: &RestartConfig, restart_count: u64) -> Duration {
    let exponent = u32::try_from(restart_count.min(20)).unwrap_or(20);
    let multiplier = 2_u64.saturating_pow(exponent);
    let milliseconds = config
        .initial_delay_ms
        .saturating_mul(multiplier)
        .min(config.maximum_delay_seconds.saturating_mul(1_000));
    Duration::from_millis(milliseconds)
}

#[cfg(test)]
mod tests {
    use super::restart_delay;
    use crate::config::RestartConfig;
    use std::time::Duration;

    #[test]
    fn restart_backoff_is_exponential_and_bounded() {
        let config = RestartConfig {
            enabled: true,
            initial_delay_ms: 100,
            maximum_delay_seconds: 1,
            maximum_restarts_per_minute: 5,
        };
        assert_eq!(restart_delay(&config, 0), Duration::from_millis(100));
        assert_eq!(restart_delay(&config, 2), Duration::from_millis(400));
        assert_eq!(restart_delay(&config, 20), Duration::from_secs(1));
    }
}
