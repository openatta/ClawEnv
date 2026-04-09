use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::{ConfigManager, InstanceConfig};

/// 启动状态——决定进入哪个页面
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchState {
    /// 首次运行，无配置文件
    FirstRun,
    /// OpenClaw 未安装（有配置但无实例）
    NotInstalled,
    /// 已安装，有可用升级
    UpgradeAvailable {
        instances: Vec<InstanceConfig>,
    },
    /// 已安装，一切正常
    Ready {
        instances: Vec<InstanceConfig>,
    },
}

/// 启动检测——在 Tauri setup 阶段调用
pub async fn detect_launch_state() -> Result<LaunchState> {
    // 1. 检查配置文件是否存在
    if !ConfigManager::exists()? {
        return Ok(LaunchState::FirstRun);
    }

    // 2. 读取配置，获取实例列表
    let config = ConfigManager::load()?;
    let instances = config.instances().to_vec();

    if instances.is_empty() {
        return Ok(LaunchState::NotInstalled);
    }

    // 3. 检查升级（超时 3 秒，失败则忽略）
    let has_upgrade = tokio::time::timeout(
        Duration::from_secs(3),
        check_upgrade_available(&instances[0]),
    )
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);

    if has_upgrade {
        Ok(LaunchState::UpgradeAvailable { instances })
    } else {
        Ok(LaunchState::Ready { instances })
    }
}

async fn check_upgrade_available(instance: &InstanceConfig) -> Result<bool> {
    let info = crate::update::checker::check_latest_version(&instance.version).await?;
    Ok(info.has_upgrade)
}

/// Auto-start OpenClaw after installation completes.
/// Called after install wizard finishes successfully.
pub async fn post_install_start(instance: &InstanceConfig) -> Result<()> {
    use crate::manager::instance::{backend_for_instance, start_instance};
    use crate::monitor::InstanceMonitor;

    let backend = backend_for_instance(instance)?;
    backend.start().await?;
    backend.exec("openclaw start --daemon").await?;

    // Wait for health check (up to 30 seconds)
    for _ in 0..30 {
        let health = InstanceMonitor::check_health(backend.as_ref()).await;
        if health == crate::monitor::InstanceHealth::Running {
            tracing::info!("OpenClaw started successfully for '{}'", instance.name);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    anyhow::bail!("OpenClaw failed to start within 30 seconds")
}
