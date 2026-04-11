use clawenv_core::claw::ClawRegistry;
use clawenv_core::config::ConfigManager;
use clawenv_core::launcher::{self, LaunchState};
use clawenv_core::manager::instance;
use serde::Serialize;
use tauri::{Manager, webview::WebviewWindowBuilder};

#[tauri::command]
pub async fn detect_launch_state() -> Result<LaunchState, String> {
    launcher::detect_launch_state()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_openclaw_url(instance_name: Option<String>) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let name = instance_name.unwrap_or_else(|| "default".into());

    let inst = config
        .instances()
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| format!("Instance '{}' not found", name))?;

    Ok(format!("http://127.0.0.1:{}", inst.gateway.gateway_port))
}

#[derive(Debug, Serialize)]
pub struct InstanceInfo {
    pub name: String,
    pub claw_type: String,
    pub display_name: String,
    pub logo: String,
    pub sandbox_type: String,
    pub version: String,
    pub gateway_port: u16,
    pub ttyd_port: u16,
}

#[tauri::command]
pub fn list_instances() -> Result<Vec<InstanceInfo>, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let registry = ClawRegistry::load();

    Ok(config
        .instances()
        .iter()
        .map(|inst| {
            let desc = registry.get(&inst.claw_type);
            InstanceInfo {
                name: inst.name.clone(),
                claw_type: inst.claw_type.clone(),
                display_name: desc.display_name.clone(),
                logo: desc.logo.clone(),
                sandbox_type: format!("{:?}", inst.sandbox_type),
                version: inst.version.clone(),
                gateway_port: inst.gateway.gateway_port,
                ttyd_port: inst.gateway.ttyd_port,
            }
        })
        .collect())
}

#[derive(Serialize)]
pub struct InstanceStatusDetail {
    pub processes: String,
    pub resources: String,
    pub gateway_log: String,
}

#[tauri::command]
pub async fn get_instance_status_detail(name: String) -> Result<InstanceStatusDetail, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    let processes = backend.exec(
        "ps aux 2>/dev/null || ps -ef 2>/dev/null || echo 'ps not available'"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));

    let resources = backend.exec(
        "echo '--- Memory ---' && free -m 2>/dev/null || cat /proc/meminfo 2>/dev/null | head -5; echo ''; echo '--- Disk ---' && df -h / 2>/dev/null; echo ''; echo '--- Uptime ---' && uptime 2>/dev/null"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));

    // Read gateway log — uses the unified log path from install/instance management
    let gateway_log = backend.exec(
        "tail -100 /tmp/clawenv-gateway.log 2>/dev/null || echo 'No gateway log found'"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));

    Ok(InstanceStatusDetail { processes, resources, gateway_log })
}

#[tauri::command]
pub async fn get_instance_logs(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let log = backend.exec(
        "tail -100 /tmp/clawenv-gateway.log 2>/dev/null || echo 'No gateway log'"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));
    Ok(log)
}

#[tauri::command]
pub async fn open_install_window(app: tauri::AppHandle, instance_name: Option<String>, claw_type: Option<String>) -> Result<(), String> {
    let name = instance_name.unwrap_or_else(|| "default".into());
    let ct = claw_type.unwrap_or_else(|| "openclaw".into());
    let registry = ClawRegistry::load();
    let desc = registry.get(&ct);
    let label = format!("install-{name}");
    let url = format!("/index.html?mode=install&name={name}&clawType={ct}");

    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
        .title(format!("Install {} — {name}", desc.display_name))
        .inner_size(900.0, 650.0)
        .resizable(true)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn get_sandbox_ip(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    instance::get_sandbox_ip(inst).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_instance(name: String) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    instance::start_instance(inst).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_instance(name: String) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    instance::stop_instance(inst).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_instance(name: String) -> Result<(), String> {
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;
    instance::remove_instance(&mut config, &name).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_instance(old_name: String, new_name: String) -> Result<(), String> {
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &old_name).map_err(|e| e.to_string())?.clone();
    let backend = instance::backend_for_instance(&inst).map_err(|e| e.to_string())?;

    // Stop first if running
    instance::stop_instance(&inst).await.ok();

    // Rename in backend (Lima: limactl rename)
    let new_sandbox_id = if backend.supports_rename() {
        backend.rename(&new_name).await.map_err(|e| e.to_string())?
    } else {
        format!("{:?}-{}", inst.sandbox_type, new_name).to_lowercase()
    };

    // Update config
    if let Some(entry) = config.config_mut().instances.iter_mut().find(|i| i.name == old_name) {
        entry.name = new_name.clone();
        entry.sandbox_id = new_sandbox_id;
    }
    config.save().map_err(|e| e.to_string())?;

    // Rename workspace dir
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let old_ws = std::path::PathBuf::from(&home).join(format!(".clawenv/workspaces/{old_name}"));
    let new_ws = std::path::PathBuf::from(&home).join(format!(".clawenv/workspaces/{new_name}"));
    if old_ws.exists() {
        let _ = tokio::fs::rename(&old_ws, &new_ws).await;
    }

    Ok(())
}

#[tauri::command]
pub async fn edit_instance_resources(
    name: String,
    cpus: Option<u32>,
    memory_mb: Option<u32>,
    disk_gb: Option<u32>,
) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    if !backend.supports_resource_edit() {
        return Err("This backend does not support resource editing".into());
    }

    // Must stop before editing
    instance::stop_instance(inst).await.ok();
    backend.edit_resources(cpus, memory_mb, disk_gb).await.map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn edit_instance_ports(
    name: String,
    gateway_port: u16,
    ttyd_port: u16,
) -> Result<(), String> {
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    if backend.supports_port_edit() {
        instance::stop_instance(inst).await.ok();
        backend.edit_port_forwards(&[(gateway_port, gateway_port), (ttyd_port, ttyd_port)])
            .await.map_err(|e| e.to_string())?;
    }

    // Update config
    if let Some(entry) = config.config_mut().instances.iter_mut().find(|i| i.name == name) {
        entry.gateway.gateway_port = gateway_port;
        entry.gateway.ttyd_port = ttyd_port;
    }
    config.save().map_err(|e| e.to_string())?;

    Ok(())
}

/// Get backend capabilities for an instance
#[tauri::command]
pub async fn get_instance_capabilities(name: String) -> Result<serde_json::Value, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "rename": backend.supports_rename(),
        "resource_edit": backend.supports_resource_edit(),
        "port_edit": backend.supports_port_edit(),
    }))
}

#[tauri::command]
pub async fn get_instance_health(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let health = instance::instance_health(inst).await;
    // Return snake_case to match serde serialization in monitor events
    let result = match health {
        clawenv_core::monitor::InstanceHealth::Running => "running",
        clawenv_core::monitor::InstanceHealth::Stopped => "stopped",
        clawenv_core::monitor::InstanceHealth::Unreachable => "unreachable",
    };
    tracing::info!("get_instance_health('{}') = {}", name, result);
    Ok(result.to_string())
}
