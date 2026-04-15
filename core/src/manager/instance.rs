use anyhow::{anyhow, Result};

use crate::claw::ClawRegistry;
use crate::config::{ConfigManager, InstanceConfig};
use crate::monitor::{InstanceHealth, InstanceMonitor};
use crate::platform::{network, process};
use crate::sandbox::{
    native_backend, LimaBackend, PodmanBackend, WslBackend, SandboxBackend, SandboxType,
};

/// Get the appropriate sandbox backend for an instance
pub fn backend_for_instance(instance: &InstanceConfig) -> Result<Box<dyn SandboxBackend>> {
    // Use sandbox_id as the actual VM/container/directory name (ASCII-safe)
    let id = &instance.sandbox_id;
    match instance.sandbox_type {
        SandboxType::LimaAlpine => Ok(Box::new(LimaBackend::new_with_vm_name(id))),
        SandboxType::Wsl2Alpine => Ok(Box::new(WslBackend::new_with_distro_name(id))),
        SandboxType::PodmanAlpine => Ok(Box::new(PodmanBackend::with_port(id, instance.gateway.gateway_port))),
        SandboxType::Native => {
            // Native: single instance, fixed directory ~/.clawenv/native/
            Ok(Box::new(native_backend("native")))
        }
    }
}

/// Start an OpenClaw instance
pub async fn start_instance(instance: &InstanceConfig) -> Result<()> {
    // For native mode, ensure Node.js and claw binaries are in PATH
    if instance.sandbox_type == SandboxType::Native {
        // Ensure ClawEnv's own Node.js is in PATH (also done inside NativeBackend::exec)
        crate::manager::install_native::ensure_node_in_path();
    }

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

    let registry = ClawRegistry::load();
    let desc = registry.get(&instance.claw_type);
    let port = instance.gateway.gateway_port;

    // Helper: check if a port is listening (cross-platform)
    async fn is_port_listening(backend: &dyn crate::sandbox::SandboxBackend, port: u16, is_native_windows: bool) -> bool {
        let cmd = if is_native_windows {
            format!("powershell -Command \"if (Test-NetConnection -ComputerName 127.0.0.1 -Port {port} -InformationLevel Quiet -WarningAction SilentlyContinue) {{ echo yes }} else {{ echo no }}\"")
        } else {
            format!("netstat -tlnp 2>/dev/null | grep -q ':{port} ' && echo yes || echo no")
        };
        let check = backend.exec(&cmd).await.unwrap_or_default();
        check.trim().contains("yes") || check.trim() == "True"
    }

    // Start ttyd (sandbox only) — check by port, not by pgrep
    if instance.sandbox_type != SandboxType::Native {
        let ttyd_port = instance.gateway.ttyd_port;
        if !is_port_listening(backend.as_ref(), ttyd_port, false).await {
            // Kill any stale ttyd first
            backend.exec("pkill -9 ttyd 2>/dev/null || true").await.ok();
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            backend.exec(&format!(
                "nohup ttyd -p {ttyd_port} -W -i 0.0.0.0 sh -c 'cd; exec /bin/sh -l' > /tmp/ttyd.log 2>&1 &"
            )).await?;
            tracing::info!("ttyd started on 0.0.0.0:{ttyd_port}");
        } else {
            tracing::info!("ttyd already listening on {ttyd_port}, skipping");
        }
    }

    // Check if gateway is already responding on this port — skip restart if already running
    let is_win_native = cfg!(target_os = "windows") && instance.sandbox_type == SandboxType::Native;
    if is_port_listening(backend.as_ref(), port, is_win_native).await {
        tracing::info!("Gateway already running on port {port}, skipping restart");
        return Ok(());
    }

    // Kill stale gateway processes then start fresh
    for pn in &desc.process_names() {
        backend.exec(&process::kill_by_name_cmd(pn)).await.ok();
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // For Lima: bind gateway on 0.0.0.0 so guestagent can detect and forward the port.
    // Use --allow-unconfigured flag instead of config set to avoid ConfigMutationConflictError.
    // Port is passed via --port flag in gateway_start_cmd, no need to modify openclaw config.
    if instance.sandbox_type == SandboxType::LimaAlpine {
        backend.exec(&format!(
            "{bin} config set gateway.bind lan 2>/dev/null; {bin} config set gateway.controlUi.dangerouslyAllowHostHeaderOriginFallback true 2>/dev/null || true",
            bin = desc.cli_binary
        )).await.ok();
        // Brief pause to let config flush
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let gateway_cmd = desc.gateway_start_cmd(port);
    #[cfg(not(target_os = "windows"))]
    backend.exec(&format!(
        "nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
    )).await?;
    #[cfg(target_os = "windows")]
    {
        if instance.sandbox_type == SandboxType::Native {
            // Windows native: launch gateway as a hidden detached process via cmd.exe.
            // NativeBackend::exec already injects ClawEnv node PATH.
            let log_path = dirs::home_dir().unwrap_or_default()
                .join(".clawenv").join("native").join("gateway.log");
            let log = log_path.display().to_string().replace('\'', "''");
            let full_cmd = format!("{} > \"{}\" 2>&1",
                gateway_cmd.replace('\'', "''"), log);
            backend.exec(&format!(
                "Start-Process -WindowStyle Hidden -FilePath 'cmd.exe' \
                 -ArgumentList '/c {full_cmd}'"
            )).await?;
        } else {
            // Windows sandbox (WSL2): nohup works inside Linux
            backend.exec(&format!(
                "nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
            )).await?;
        }
    }

    // Wait for gateway to become responsive (up to 15s)
    for i in 0..6 {
        tokio::time::sleep(std::time::Duration::from_secs(if i == 0 { 3 } else { 2 }).into()).await;

        // Platform-appropriate HTTP probe
        #[cfg(not(target_os = "windows"))]
        let check_cmd = format!(
            "curl -s -o /dev/null -w '%{{http_code}}' --connect-timeout 2 http://127.0.0.1:{port}/ 2>/dev/null || echo '000'"
        );
        #[cfg(target_os = "windows")]
        let check_cmd = if instance.sandbox_type == SandboxType::Native {
            format!(
                "powershell -Command \"try {{ (Invoke-WebRequest -Uri http://127.0.0.1:{port}/ -TimeoutSec 2 -UseBasicParsing).StatusCode }} catch {{ '000' }}\""
            )
        } else {
            format!(
                "curl -s -o /dev/null -w '%{{http_code}}' --connect-timeout 2 http://127.0.0.1:{port}/ 2>/dev/null || echo '000'"
            )
        };

        let check = backend.exec(&check_cmd).await.unwrap_or_default();
        let code = check.trim().trim_matches('\'');
        if code != "000" && !code.is_empty() {
            tracing::info!("Instance '{}' gateway ready on port {port} (HTTP {code})", instance.name);
            return Ok(());
        }
    }

    // Gateway did not respond after 6 probes (~13s). Check if process even started.
    // Check any of the process name variants
    let mut proc_check = String::new();
    for pn in &desc.process_names() {
        let check = backend.exec(&process::check_process_cmd(pn)).await.unwrap_or_default();
        if check.contains("running") { proc_check = check; break; }
    }
    if proc_check.contains("running") {
        // Process is alive but not responding to HTTP yet — warn but don't fail.
        // It may be doing first-time initialization.
        tracing::warn!("Instance '{}' gateway process is running but not yet responding on port {port}", instance.name);
        Ok(())
    } else {
        // Process is not running — real failure
        let log = backend.exec("tail -20 /tmp/clawenv-gateway.log 2>/dev/null || echo 'no log'").await.unwrap_or_default();
        anyhow::bail!(
            "Gateway failed to start for '{}' on port {port}. \
             Process exited unexpectedly.\n\nGateway log:\n{log}",
            instance.name
        )
    }
}

/// Stop a claw instance — force kill all processes
pub async fn stop_instance(instance: &InstanceConfig) -> Result<()> {
    let registry = ClawRegistry::load();
    let desc = registry.get(&instance.claw_type);
    let backend = backend_for_instance(instance)?;
    // Force kill gateway and ttyd
    for pn in &desc.process_names() {
        backend.exec(&process::kill_by_name_cmd(pn)).await.ok();
    }
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
    let is_win_native = cfg!(target_os = "windows") && instance.sandbox_type == SandboxType::Native;
    InstanceMonitor::check_health_with_port_platform(backend.as_ref(), instance.gateway.gateway_port, is_win_native).await
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
