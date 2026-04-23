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
    /// Config file path: `<clawenv_root>/config.toml`.
    /// Honours `CLAWENV_HOME` env var via `config::clawenv_root()`.
    pub fn config_path() -> Result<PathBuf> {
        Ok(super::clawenv_root().join("config.toml"))
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
                remote: Default::default(),
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
        self.write_config(&self.config)
    }

    /// Atomic write: serialize config → write to tmp → rename.
    /// Holds exclusive lock for the entire write operation.
    fn write_config(&self, config: &AppConfig) -> Result<()> {
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(config)?;

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

    /// Add or update an instance in config with load-merge-write to avoid race conditions.
    ///
    /// Instead of using the in-memory config (which may be stale after a long install),
    /// re-reads the latest config from disk, merges the new instance, and writes atomically.
    /// Safe for concurrent processes installing different instances simultaneously.
    pub fn save_instance(&mut self, instance: super::models::InstanceConfig) -> Result<()> {
        let lock_path = self.config_path.with_extension("toml.lock");
        let lock_file = File::create(&lock_path)?;
        lock_file.lock_exclusive().map_err(|e| anyhow!("Cannot acquire config lock: {e}"))?;

        // Re-read latest config from disk (another process may have written since our load)
        let fresh_config = if self.config_path.exists() {
            let content = std::fs::read_to_string(&self.config_path)?;
            toml::from_str::<AppConfig>(&content).unwrap_or_else(|_| self.config.clone())
        } else {
            self.config.clone()
        };

        // Merge: remove old entry with same name, add new one
        let mut merged = fresh_config;
        merged.instances.retain(|i| i.name != instance.name);
        merged.instances.push(instance);

        // Write merged config
        self.write_config(&merged)?;

        // Update in-memory config to match what we wrote
        self.config = merged;

        lock_file.unlock().ok();
        let _ = std::fs::remove_file(&lock_path);
        Ok(())
    }

    /// Remove an instance from config with load-merge-write to avoid race conditions.
    pub fn remove_instance(&mut self, name: &str) -> Result<()> {
        let lock_path = self.config_path.with_extension("toml.lock");
        let lock_file = File::create(&lock_path)?;
        lock_file.lock_exclusive().map_err(|e| anyhow!("Cannot acquire config lock: {e}"))?;

        let fresh_config = if self.config_path.exists() {
            let content = std::fs::read_to_string(&self.config_path)?;
            toml::from_str::<AppConfig>(&content).unwrap_or_else(|_| self.config.clone())
        } else {
            self.config.clone()
        };

        let mut merged = fresh_config;
        merged.instances.retain(|i| i.name != name);

        self.write_config(&merged)?;
        self.config = merged;

        lock_file.unlock().ok();
        let _ = std::fs::remove_file(&lock_path);
        Ok(())
    }

    /// Update fields of an existing instance with load-merge-write.
    pub fn update_instance(&mut self, name: &str, mutate: impl FnOnce(&mut super::models::InstanceConfig)) -> Result<()> {
        let lock_path = self.config_path.with_extension("toml.lock");
        let lock_file = File::create(&lock_path)?;
        lock_file.lock_exclusive().map_err(|e| anyhow!("Cannot acquire config lock: {e}"))?;

        let fresh_config = if self.config_path.exists() {
            let content = std::fs::read_to_string(&self.config_path)?;
            toml::from_str::<AppConfig>(&content).unwrap_or_else(|_| self.config.clone())
        } else {
            self.config.clone()
        };

        let mut merged = fresh_config;
        if let Some(inst) = merged.instances.iter_mut().find(|i| i.name == name) {
            mutate(inst);
        }

        self.write_config(&merged)?;
        self.config = merged;

        lock_file.unlock().ok();
        let _ = std::fs::remove_file(&lock_path);
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
