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

    Ok(format!("http://127.0.0.1:{}", inst.openclaw.gateway_port))
}

#[derive(Debug, Serialize)]
pub struct InstanceInfo {
    pub name: String,
    pub sandbox_type: String,
    pub version: String,
    pub gateway_port: u16,
    pub ttyd_port: u16,
}

#[tauri::command]
pub fn list_instances() -> Result<Vec<InstanceInfo>, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;

    Ok(config
        .instances()
        .iter()
        .map(|inst| InstanceInfo {
            name: inst.name.clone(),
            sandbox_type: format!("{:?}", inst.sandbox_type),
            version: inst.version.clone(),
            gateway_port: inst.openclaw.gateway_port,
            ttyd_port: inst.openclaw.ttyd_port,
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

    // Read the actual running gateway log (not the startup wrapper log)
    let gateway_log = backend.exec(
        "cat /tmp/openclaw/openclaw-*.log 2>/dev/null | tail -100 || tail -80 /tmp/openclaw-gateway.log 2>/dev/null || echo 'No gateway log found'"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));

    Ok(InstanceStatusDetail { processes, resources, gateway_log })
}

#[tauri::command]
pub async fn get_instance_logs(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let log = backend.exec(
        "cat /tmp/openclaw/openclaw-*.log 2>/dev/null | tail -100 || tail -100 /tmp/openclaw-gateway.log 2>/dev/null || echo 'No gateway log'"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));
    Ok(log)
}

#[tauri::command]
pub async fn open_install_window(app: tauri::AppHandle, instance_name: Option<String>) -> Result<(), String> {
    let name = instance_name.unwrap_or_else(|| "default".into());
    let label = format!("install-{name}");
    let url = format!("/index.html?mode=install&name={name}");

    // If window already exists, focus it
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
        .title(format!("Install OpenClaw — {name}"))
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
