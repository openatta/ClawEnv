use anyhow::{anyhow, Result};
use std::path::PathBuf;

use super::models::AppConfig;

pub struct ConfigManager {
    config_path: PathBuf,
    config: AppConfig,
}

impl ConfigManager {
    /// 配置文件路径: ~/.clawenv/config.toml
    pub fn config_path() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/config.toml"))
    }

    /// 配置文件是否存在
    pub fn exists() -> Result<bool> {
        Ok(Self::config_path()?.exists())
    }

    /// 加载配置。如果文件不存在，返回错误；如果文件存在但解析失败（损坏），
    /// 创建 .toml.bak 备份，记录警告日志，并重建默认配置。
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;
        if !config_path.exists() {
            return Err(anyhow!("Config file not found: {}", config_path.display()));
        }
        let content = std::fs::read_to_string(&config_path)?;
        match toml::from_str::<AppConfig>(&content) {
            Ok(config) => Ok(Self { config_path, config }),
            Err(parse_err) => {
                // Config is corrupted — backup and recreate
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

    /// 加载配置，如果文件不存在则创建默认配置
    pub fn load_or_default(mode: super::models::UserMode) -> Result<Self> {
        match Self::load() {
            Ok(mgr) => Ok(mgr),
            Err(_) if !Self::config_path()?.exists() => Self::create_default(mode),
            Err(e) => Err(e),
        }
    }

    /// 创建默认配置并保存
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
            },
            instances: vec![],
        };

        let manager = Self { config_path, config };
        manager.save()?;
        Ok(manager)
    }

    /// 保存配置到文件
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(&self.config)?;
        std::fs::write(&self.config_path, content)?;
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
