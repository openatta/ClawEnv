use anyhow::{anyhow, Result};

use crate::config::{ConfigManager, InstanceConfig};
use crate::monitor::{InstanceHealth, InstanceMonitor};
use crate::platform::{network, process};
use crate::sandbox::{
    native_backend, LimaBackend, PodmanBackend, WslBackend, SandboxBackend, SandboxType,
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

    // Only call backend.start() if VM/container is not already running
    let already_running = backend.exec("echo ok").await.map(|o| o.contains("ok")).unwrap_or(false);
    if !already_running {
        backend.start().await?;
        for _ in 0..5 {
            if backend.exec("echo ok").await.map(|o| o.contains("ok")).unwrap_or(false) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    // Sync host IP (sandbox only)
    if instance.sandbox_type != SandboxType::Native {
        match network::sync_host_ip(backend.as_ref()).await {
            Ok(true) => tracing::info!("Host IP updated in sandbox '{}'", instance.name),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to sync host IP: {e}"),
        }
    }

    let port = instance.openclaw.gateway_port;

    // Start ttyd (sandbox only)
    if instance.sandbox_type != SandboxType::Native {
        let ttyd_port = instance.openclaw.ttyd_port;
        let ttyd_check = backend.exec(&process::check_process_cmd(&format!("ttyd.*-p {ttyd_port}")))
            .await.unwrap_or_default();
        if !ttyd_check.contains("running") {
            backend.exec(&format!(
                "nohup ttyd -p {ttyd_port} -W -i 0.0.0.0 sh -c 'cd; exec /bin/sh -l' > /tmp/ttyd.log 2>&1 &"
            )).await?;
            tracing::info!("ttyd started on 0.0.0.0:{ttyd_port}");
        }
    }

    // Always kill stale gateway then restart fresh
    backend.exec(&process::kill_by_name_cmd("openclaw gateway")).await.ok();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    backend.exec(&format!(
        "nohup openclaw gateway --port {port} --allow-unconfigured > /tmp/openclaw-gateway.log 2>&1 &"
    )).await?;

    // Wait for gateway to become responsive (up to 15s)
    for i in 0..6 {
        tokio::time::sleep(std::time::Duration::from_secs(if i == 0 { 3 } else { 2 }).into()).await;
        let check = backend.exec(&format!(
            "curl -s -o /dev/null -w '%{{http_code}}' --connect-timeout 2 http://127.0.0.1:{port}/ 2>/dev/null || echo '000'"
        )).await.unwrap_or_default();
        let code = check.trim().trim_matches('\'');
        if code != "000" && !code.is_empty() {
            tracing::info!("Instance '{}' gateway ready on port {port} (HTTP {code})", instance.name);
            return Ok(());
        }
    }

    tracing::warn!("Instance '{}' gateway started but not yet responding on port {port}", instance.name);
    Ok(())
}

/// Stop an OpenClaw instance — force kill all processes
pub async fn stop_instance(instance: &InstanceConfig) -> Result<()> {
    let backend = backend_for_instance(instance)?;
    // Force kill gateway and ttyd
    backend.exec(&process::kill_by_name_cmd("openclaw gateway")).await.ok();
    backend.exec(&process::kill_by_name_cmd("ttyd")).await.ok();
    backend.stop().await?;
    tracing::info!("Instance '{}' stopped", instance.name);
    Ok(())
}

/// Restart an OpenClaw instance
pub async fn restart_instance(instance: &InstanceConfig) -> Result<()> {
    stop_instance(instance).await.ok();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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

/// Get the VM's external IP address
pub async fn get_sandbox_ip(instance: &InstanceConfig) -> Result<String> {
    if instance.sandbox_type == SandboxType::Native {
        return Ok("127.0.0.1".into());
    }
    let backend = backend_for_instance(instance)?;
    let output = backend.exec(
        "ip -4 addr show eth0 2>/dev/null | grep -oP 'inet \\K[0-9.]+' || hostname -i 2>/dev/null || echo '127.0.0.1'"
    ).await?;
    let ip = output.trim().to_string();
    if ip.is_empty() { Ok("127.0.0.1".into()) } else { Ok(ip) }
}
