use anyhow::{anyhow, Result};

use crate::config::{ConfigManager, InstanceConfig};
use crate::monitor::{InstanceHealth, InstanceMonitor};
use crate::sandbox::{
    detect_backend, native_backend, LimaBackend, PodmanBackend, WslBackend, SandboxBackend, SandboxType,
};

/// Get the appropriate sandbox backend for an instance
pub fn backend_for_instance(instance: &InstanceConfig) -> Result<Box<dyn SandboxBackend>> {
    match instance.sandbox_type {
        SandboxType::LimaAlpine => Ok(Box::new(LimaBackend::new(&instance.name))),
        SandboxType::Wsl2Alpine => Ok(Box::new(WslBackend::new(&instance.name))),
        SandboxType::PodmanAlpine => Ok(Box::new(PodmanBackend::with_defaults(&instance.name))),
        SandboxType::Native => Ok(Box::new(native_backend(&instance.name))),
    }
}

/// Start an OpenClaw instance
pub async fn start_instance(instance: &InstanceConfig) -> Result<()> {
    let backend = backend_for_instance(instance)?;
    backend.start().await?;
    backend.exec("openclaw start --daemon").await?;
    tracing::info!("Instance '{}' started", instance.name);
    Ok(())
}

/// Stop an OpenClaw instance
pub async fn stop_instance(instance: &InstanceConfig) -> Result<()> {
    let backend = backend_for_instance(instance)?;
    backend.exec("openclaw stop").await.ok(); // best-effort stop openclaw
    backend.stop().await?;
    tracing::info!("Instance '{}' stopped", instance.name);
    Ok(())
}

/// Restart an OpenClaw instance
pub async fn restart_instance(instance: &InstanceConfig) -> Result<()> {
    stop_instance(instance).await.ok();
    start_instance(instance).await
}

/// Get the health status of an instance
pub async fn instance_health(instance: &InstanceConfig) -> InstanceHealth {
    let backend = match backend_for_instance(instance) {
        Ok(b) => b,
        Err(_) => return InstanceHealth::Unreachable,
    };
    InstanceMonitor::check_health(backend.as_ref()).await
}

/// Get instance by name from config
pub fn get_instance<'a>(config: &'a ConfigManager, name: &str) -> Result<&'a InstanceConfig> {
    config
        .instances()
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| anyhow!("Instance '{}' not found", name))
}

/// Remove an instance from config and destroy its sandbox
pub async fn remove_instance(config: &mut ConfigManager, name: &str) -> Result<()> {
    let instance = config
        .instances()
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| anyhow!("Instance '{}' not found", name))?
        .clone();

    let backend = backend_for_instance(&instance)?;
    stop_instance(&instance).await.ok();
    backend.destroy().await?;

    config.config_mut().instances.retain(|i| i.name != name);
    config.save()?;

    tracing::info!("Instance '{}' removed", name);
    Ok(())
}
