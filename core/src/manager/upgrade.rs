use anyhow::Result;

use crate::config::{ConfigManager, InstanceConfig};
use crate::manager::instance::backend_for_instance;
use crate::update::checker::{self, VersionInfo};

/// Check if an upgrade is available for an instance
pub async fn check_upgrade(instance: &InstanceConfig) -> Result<VersionInfo> {
    checker::check_latest_version(&instance.version).await
}

/// Upgrade an instance to a target version (or latest)
pub async fn upgrade(
    config: &mut ConfigManager,
    instance_name: &str,
    target_version: Option<&str>,
) -> Result<String> {
    let instance = config
        .instances()
        .iter()
        .find(|i| i.name == instance_name)
        .ok_or_else(|| anyhow::anyhow!("Instance '{}' not found", instance_name))?
        .clone();

    let backend = backend_for_instance(&instance)?;

    // 1. Pre-upgrade snapshot
    let snap_tag = format!("pre-upgrade-{}", instance.version);
    tracing::info!("Creating pre-upgrade snapshot: {snap_tag}");
    backend.snapshot_create(&snap_tag).await?;

    // 2. Perform upgrade inside sandbox
    let version = target_version.unwrap_or("latest");
    tracing::info!("Upgrading OpenClaw to {version}...");
    backend
        .exec(&format!("npm update -g openclaw@{version}"))
        .await?;

    // 3. Verify new version
    let new_ver = backend.exec("openclaw --version").await?;
    let new_ver = new_ver.trim().to_string();
    tracing::info!("Upgraded to {new_ver}");

    // 4. Update config
    for inst in config.config_mut().instances.iter_mut() {
        if inst.name == instance_name {
            inst.version = new_ver.clone();
            inst.last_upgraded_at = chrono::Utc::now().to_rfc3339();
        }
    }
    config.save()?;

    Ok(new_ver)
}

/// Rollback to a snapshot
pub async fn rollback(instance: &InstanceConfig, tag: &str) -> Result<()> {
    let backend = backend_for_instance(instance)?;
    tracing::info!("Rolling back instance '{}' to snapshot '{tag}'", instance.name);
    backend.snapshot_restore(tag).await?;
    Ok(())
}
