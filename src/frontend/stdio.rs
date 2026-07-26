use std::sync::Arc;

use anyhow::{Context, Result};
use rmcp::{ServiceExt, transport::stdio};

use super::HydianServer;
use crate::runtime::Runtime;

pub async fn serve(runtime: Arc<Runtime>) -> Result<()> {
    let service = HydianServer::new(runtime.clone())
        .serve(stdio())
        .await
        .context("could not initialize Hydian stdio frontend")?;
    service
        .waiting()
        .await
        .context("Hydian stdio frontend task failed")?;
    runtime.shutdown().await;
    Ok(())
}
