use anyhow::{anyhow, Result};

use crate::config::{ConfigManager, InstanceConfig};
use crate::monitor::{InstanceHealth, InstanceMonitor};
use crate::platform::network;
use crate::sandbox::{
    detect_backend, native_backend, LimaBackend, PodmanBackend, WslBackend, SandboxBackend, SandboxType,
};

/// Get the appropriate sandbox backend for an instance
pub fn backend_for_instance(instance: &InstanceConfig) -> Result<Box<dyn SandboxBackend>> {
    match instance.sandbox_type {
        SandboxType::LimaAlpine => Ok(Box::new(LimaBackend::new(&instance.name))),
        SandboxType::Wsl2Alpine => Ok(Box::new(WslBackend::new(&instance.name))),
        SandboxType::PodmanAlpine => Ok(Box::new(PodmanBackend::with_port(&instance.name, instance.openclaw.gateway_port))),
        SandboxType::Native => Ok(Box::new(native_backend(&instance.name))),
    }
}

/// Start an OpenClaw instance
pub async fn start_instance(instance: &InstanceConfig) -> Result<()> {
    let backend = backend_for_instance(instance)?;
    backend.start().await?;

    // Sync host IP for Bridge Server access (LAN IP may change between reboots/networks)
    // Skip for Native backend (no VM isolation, host IS the sandbox)
    if instance.sandbox_type != SandboxType::Native {
        match network::sync_host_ip(backend.as_ref()).await {
            Ok(true) => tracing::info!("Host IP updated in sandbox '{}'", instance.name),
            Ok(false) => {} // unchanged
            Err(e) => tracing::warn!("Failed to sync host IP: {e}"),
        }
    }

    let port = instance.openclaw.gateway_port;

    // Check if gateway is already running on this port
    let check = backend.exec(&format!(
        "curl -s -o /dev/null -w '%{{http_code}}' --connect-timeout 2 http://127.0.0.1:{port}/ 2>/dev/null || echo '000'"
    )).await.unwrap_or_default();
    let code = check.trim().trim_matches('\'');

    if code.starts_with('2') || code.starts_with('3') || code == "401" {
        tracing::info!("Instance '{}' gateway already running on port {port}", instance.name);
        return Ok(());
    }

    // Start gateway
    backend.exec(&format!(
        "nohup openclaw gateway --port {port} --allow-unconfigured > /tmp/openclaw-gateway.log 2>&1 &"
    )).await?;
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
    InstanceMonitor::check_health_with_port(backend.as_ref(), instance.openclaw.gateway_port).await
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
