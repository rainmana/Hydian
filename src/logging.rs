use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::{HydianConfig, LogFormat};

pub fn init(config: &HydianConfig, logs: &Path) -> Result<WorkerGuard> {
    fs::create_dir_all(logs)
        .with_context(|| format!("could not create log directory {}", logs.display()))?;
    enforce_retention(logs, u64::from(config.logging.retain_days))?;
    let appender = tracing_appender::rolling::daily(logs, "hydian.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_new(&config.logging.level)
        .with_context(|| format!("invalid logging level `{}`", config.logging.level))?;
    match config.logging.format {
        LogFormat::Pretty => {
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .with_ansi(false)
                        .with_target(true)
                        .with_writer(writer),
                )
                .try_init();
        }
        LogFormat::Json => {
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .json()
                        .with_ansi(false)
                        .with_target(true)
                        .with_writer(writer),
                )
                .try_init();
        }
    }
    Ok(guard)
}

pub fn enforce_retention(logs: &Path, retain_days: u64) -> Result<Vec<PathBuf>> {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(retain_days.saturating_mul(86_400)))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut removed = Vec::new();
    if !logs.exists() {
        return Ok(removed);
    }
    for entry in fs::read_dir(logs)
        .with_context(|| format!("could not inspect log directory {}", logs.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file()
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with("hydian.log")
        {
            continue;
        }
        let modified = entry.metadata()?.modified()?;
        if modified < cutoff {
            fs::remove_file(&path)
                .with_context(|| format!("could not remove expired log {}", path.display()))?;
            removed.push(path);
        }
    }
    Ok(removed)
}

#[must_use]
pub fn latest_log_file(logs: &Path) -> Option<PathBuf> {
    fs::read_dir(logs)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("hydian.log")
        })
        .max_by_key(|entry| {
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
        })
        .map(|entry| entry.path())
}
