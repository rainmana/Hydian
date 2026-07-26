use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use directories::BaseDirs;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Unix,
}

impl Platform {
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HydianPaths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub mcp_config: PathBuf,
    pub logs: PathBuf,
    pub run: PathBuf,
    pub backups: PathBuf,
    pub status: PathBuf,
}

impl HydianPaths {
    pub fn resolve(
        home_override: Option<&Path>,
        config_override: Option<&Path>,
        mcp_override: Option<&Path>,
    ) -> Result<Self> {
        let base_dirs = BaseDirs::new()
            .ok_or_else(|| anyhow!("operating-system directories are unavailable"))?;

        let home = match home_override {
            Some(path) => absolutize(path)?,
            None => match env::var_os("HYDIAN_HOME") {
                Some(value) => absolutize(Path::new(&value))?,
                None => default_home(
                    Platform::current(),
                    base_dirs.home_dir(),
                    base_dirs.data_local_dir(),
                ),
            },
        };

        let config = config_override
            .map(absolutize)
            .transpose()?
            .unwrap_or_else(|| home.join("config.toml"));
        let mcp_config = mcp_override
            .map(absolutize)
            .transpose()?
            .unwrap_or_else(|| home.join("mcp.json"));
        let logs = home.join("logs");
        let run = home.join("run");
        let backups = home.join("backups");
        let status = run.join("status.json");

        Ok(Self {
            home,
            config,
            mcp_config,
            logs,
            run,
            backups,
            status,
        })
    }

    pub fn create_directories(&self) -> Result<()> {
        for path in [&self.home, &self.logs, &self.run, &self.backups] {
            std::fs::create_dir_all(path)
                .with_context(|| format!("could not create {}", path.display()))?;
        }
        Ok(())
    }
}

#[must_use]
pub fn default_home(platform: Platform, home: &Path, local_data: &Path) -> PathBuf {
    match platform {
        Platform::Windows => local_data.join("Hydian"),
        Platform::Unix => home.join(".hydian"),
    }
}

pub fn absolutize(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(env::current_dir()
        .context("could not determine the current directory")?
        .join(path))
}

#[cfg(test)]
mod tests {
    use super::{Platform, default_home};
    use std::path::Path;

    #[test]
    fn windows_layout_uses_local_app_data() {
        assert_eq!(
            default_home(
                Platform::Windows,
                Path::new(r"C:\Users\Alec"),
                Path::new(r"C:\Users\Alec\AppData\Local")
            ),
            Path::new(r"C:\Users\Alec\AppData\Local").join("Hydian")
        );
    }

    #[test]
    fn unix_layout_is_hidden_home_directory() {
        assert_eq!(
            default_home(
                Platform::Unix,
                Path::new("/Users/alec"),
                Path::new("/Users/alec/Library/Application Support")
            ),
            Path::new("/Users/alec/.hydian")
        );
    }
}
