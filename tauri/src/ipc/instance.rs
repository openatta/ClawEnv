use clawenv_core::api::{ListResponse, StatusResponse};
use clawenv_core::claw::ClawRegistry;
use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
use serde::Serialize;
use tauri::{Emitter, Manager, webview::WebviewWindowBuilder};

use crate::cli_bridge;

#[tauri::command]
pub async fn detect_launch_state() -> Result<clawenv_core::launcher::LaunchState, String> {
    clawenv_core::launcher::detect_launch_state()
        .await
        .map_err(|e| e.to_string())
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
pub async fn list_instances() -> Result<Vec<InstanceInfo>, String> {
    let data = cli_bridge::run_cli(&["list"]).await.map_err(|e| e.to_string())?;
    let resp: ListResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;

    let registry = ClawRegistry::load();
    let instances = resp.instances.into_iter().map(|s| {
        let desc = registry.get(&s.claw_type);
        InstanceInfo {
            name: s.name,
            claw_type: s.claw_type,
            display_name: desc.display_name.clone(),
            logo: desc.logo.clone(),
            sandbox_type: s.sandbox_type,
            version: s.version,
            gateway_port: s.gateway_port,
            ttyd_port: s.ttyd_port,
        }
    }).collect();

    Ok(instances)
}

#[tauri::command]
pub async fn get_instance_logs(name: String) -> Result<String, String> {
    let data = cli_bridge::run_cli(&["logs", &name]).await.map_err(|e| e.to_string())?;
    Ok(data.as_str().unwrap_or("").to_string())
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
pub async fn start_instance(name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["start", &name]).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn stop_instance(name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["stop", &name]).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["uninstall", "--name", &name]).await.map_err(|e| e.to_string())?;
    let _ = app.emit("instances-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn rename_instance(old_name: String, new_name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["rename", &old_name, &new_name]).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn edit_instance_resources(
    name: String,
    cpus: Option<u32>,
    memory_mb: Option<u32>,
    disk_gb: Option<u32>,
) -> Result<(), String> {
    let mut args = vec!["edit".to_string(), name];
    if let Some(c) = cpus { args.extend(["--cpus".into(), c.to_string()]); }
    if let Some(m) = memory_mb { args.extend(["--memory".into(), m.to_string()]); }
    if let Some(d) = disk_gb { args.extend(["--disk".into(), d.to_string()]); }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cli_bridge::run_cli(&refs).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn edit_instance_ports(
    name: String,
    gateway_port: u16,
    ttyd_port: u16,
) -> Result<(), String> {
    cli_bridge::run_cli(&[
        "edit", &name,
        "--gateway-port", &gateway_port.to_string(),
        "--ttyd-port", &ttyd_port.to_string(),
    ]).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_instance_capabilities(name: String) -> Result<serde_json::Value, String> {
    // Capabilities are backend-specific — keep direct core call (lightweight, no subprocess needed)
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
    let data = cli_bridge::run_cli(&["status", &name]).await.map_err(|e| e.to_string())?;
    let resp: StatusResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp.health)
}

#[tauri::command]
pub fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}
