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
    let port = instance.openclaw.gateway_port;
    backend.exec(&format!(
        "nohup openclaw gateway --port {port} --allow-unconfigured > /tmp/openclaw-gateway.log 2>&1 &"
    )).await?;
    // Wait for gateway to start
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    tracing::info!("Instance '{}' started on port {port}", instance.name);
    Ok(())
}

/// Stop an OpenClaw instance
pub async fn stop_instance(instance: &InstanceConfig) -> Result<()> {
    let backend = backend_for_instance(instance)?;
    backend.exec("pkill -f 'openclaw gateway' 2>/dev/null || true").await.ok();
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
