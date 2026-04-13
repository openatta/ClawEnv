use anyhow::{anyhow, Result};
use fs2::FileExt;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use super::models::AppConfig;

pub struct ConfigManager {
    config_path: PathBuf,
    config: AppConfig,
}

impl ConfigManager {
    /// Config file path: ~/.clawenv/config.toml
    pub fn config_path() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/config.toml"))
    }

    pub fn exists() -> Result<bool> {
        Ok(Self::config_path()?.exists())
    }

    /// Load config with shared (read) file lock.
    /// If file is corrupted, backs up to .toml.bak and recreates default.
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;
        if !config_path.exists() {
            return Err(anyhow!("Config file not found: {}", config_path.display()));
        }

        // Shared lock for reading — allows concurrent readers, blocks writers
        let file = File::open(&config_path)?;
        file.lock_shared().map_err(|e| anyhow!("Cannot lock config for reading: {e}"))?;
        let content = std::fs::read_to_string(&config_path)?;
        file.unlock().ok();

        match toml::from_str::<AppConfig>(&content) {
            Ok(config) => Ok(Self { config_path, config }),
            Err(parse_err) => {
                let backup_path = config_path.with_extension("toml.bak");
                tracing::warn!(
                    "Config file corrupted ({}), backing up to {} and recreating default",
                    parse_err,
                    backup_path.display()
                );
                let _ = std::fs::copy(&config_path, &backup_path);
                let manager = Self::create_default(super::models::UserMode::General)?;
                Ok(manager)
            }
        }
    }

    pub fn load_or_default(mode: super::models::UserMode) -> Result<Self> {
        match Self::load() {
            Ok(mgr) => Ok(mgr),
            Err(_) if !Self::config_path()?.exists() => Self::create_default(mode),
            Err(e) => Err(e),
        }
    }

    pub fn create_default(user_mode: super::models::UserMode) -> Result<Self> {
        let config_path = Self::config_path()?;

        let config = AppConfig {
            clawenv: super::models::ClawEnvConfig {
                version: env!("CARGO_PKG_VERSION").to_string(),
                user_mode,
                language: "zh-CN".into(),
                theme: "system".into(),
                updates: Default::default(),
                security: Default::default(),
                tray: Default::default(),
                proxy: Default::default(),
                mirrors: Default::default(),
                bridge: Default::default(),
            },
            instances: vec![],
        };

        let manager = Self { config_path, config };
        manager.save()?;
        Ok(manager)
    }

    /// Save config atomically: write to temp file, then rename.
    /// Uses exclusive file lock to prevent concurrent writes.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(&self.config)?;

        // Write to temp file in same directory (same filesystem for atomic rename)
        let tmp_path = self.config_path.with_extension("toml.tmp");
        {
            let mut tmp_file = File::create(&tmp_path)?;
            // Exclusive lock — blocks other writers and readers
            tmp_file.lock_exclusive().map_err(|e| anyhow!("Cannot lock config for writing: {e}"))?;
            tmp_file.write_all(content.as_bytes())?;
            tmp_file.flush()?;
            tmp_file.unlock().ok();
        }

        // Atomic rename (same filesystem guarantees atomicity on all platforms)
        std::fs::rename(&tmp_path, &self.config_path)?;

        Ok(())
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut AppConfig {
        &mut self.config
    }

    pub fn instances(&self) -> &[super::models::InstanceConfig] {
        &self.config.instances
    }
}
