use std::{process::Stdio, time::Duration};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use rmcp::{
    ServiceExt,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

use super::{BackendClientHandler, BackendConnector, ConnectedBackend};
use crate::{config::HydianConfig, model::McpServerDefinition};

#[derive(Debug, Default)]
pub struct StdioConnector;

#[async_trait]
impl BackendConnector for StdioConnector {
    async fn connect(
        &self,
        name: &str,
        definition: &McpServerDefinition,
        _defaults: &HydianConfig,
    ) -> Result<ConnectedBackend> {
        let executable = definition
            .command
            .as_deref()
            .ok_or_else(|| anyhow!("stdio backend has no command"))?;
        let resolved = which::which(executable)
            .with_context(|| format!("executable `{executable}` was not found on PATH"))?;
        let mut command = Command::new(&resolved).configure(|command| {
            command.args(&definition.args);
            command.envs(&definition.env);
            if let Some(cwd) = &definition.cwd {
                command.current_dir(cwd);
            }
            command.kill_on_drop(true);
        });

        // Argument boundaries are passed directly to Command; no shell is involved.
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        let (transport, stderr) = TokioChildProcess::builder(command)
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("could not start `{}`", resolved.display()))?;
        let pid = transport.id();
        if let Some(stderr) = stderr {
            let backend = name.to_owned();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            tracing::warn!(backend = %backend, stream = "stderr", message = %line);
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(backend = %backend, %error, "could not read backend stderr");
                            break;
                        }
                    }
                }
            });
        }

        let handler = BackendClientHandler::new();
        let tools_changed = handler.change_flag();
        let service = tokio::time::timeout(
            Duration::from_secs(definition.startup_timeout_seconds.unwrap_or(20)),
            handler.serve(transport),
        )
        .await
        .map_err(|_| anyhow!("MCP initialize handshake timed out"))?
        .context("MCP initialize handshake failed")?;

        Ok(ConnectedBackend {
            service,
            pid,
            tools_changed,
        })
    }
}
