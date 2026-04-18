use clawenv_core::api::{ListResponse, StatusResponse};
use clawenv_core::claw::ClawRegistry;
use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
use serde::Serialize;
use tauri::{Emitter, Manager, webview::WebviewWindowBuilder};

use crate::cli_bridge;
use crate::ipc::emit::{emit_instance_changed, InstanceAction, InstanceChanged};

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
    /// Dashboard port for claws that split UI from gateway (Hermes).
    /// 0 means "no dashboard; UI lives at gateway_port". Forwarded here
    /// straight from `InstanceSummary::dashboard_port`; dropping this
    /// field would force the frontend to fall back to gateway_port,
    /// which is exactly the bug that made the Hermes "Open Control
    /// Panel" button land on an empty page before v0.2.7.
    pub dashboard_port: u16,
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
            dashboard_port: s.dashboard_port,
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
pub async fn start_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["start", &name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::simple(InstanceAction::Start, &name));
    Ok(())
}

#[tauri::command]
pub async fn stop_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["stop", &name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::simple(InstanceAction::Stop, &name));
    Ok(())
}

/// Stop all instances — used by quit dialog
#[tauri::command]
pub async fn stop_all_instances() -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    for inst in config.instances() {
        let _ = clawenv_core::manager::instance::stop_instance(inst).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["uninstall", "--name", &name]).await.map_err(|e| e.to_string())?;
    // `instance-changed` is the canonical state-sync event; the legacy
    // `instances-changed` (plural) is no longer emitted — MainLayout converged
    // its refresh logic onto `instance-changed`.
    emit_instance_changed(&app, InstanceChanged::deleted(&name));
    Ok(())
}

/// Delete instance with staged progress events for UI dialog
#[tauri::command]
pub async fn delete_instance_with_progress(app: tauri::AppHandle, name: String) -> Result<(), String> {
    use clawenv_core::manager::instance;
    use clawenv_core::sandbox::SandboxType;

    let emit = |stage: &str, status: &str, msg: &str| {
        let _ = app.emit("delete-progress", serde_json::json!({
            "stage": stage, "status": status, "message": msg,
        }));
    };

    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?.clone();

    // Stage 1: Stop
    emit("stop", "active", "Stopping instance...");
    let _ = instance::stop_instance(&inst).await;
    emit("stop", "done", "Stopped");

    // Stage 2: Kill processes
    emit("kill", "active", "Killing processes...");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    if inst.sandbox_type == SandboxType::Native {
        instance::kill_native_gateway_public(inst.gateway.gateway_port).await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    emit("kill", "done", "Killed");

    // Stage 3: Delete files
    emit("delete_files", "active", "Deleting files...");
    let backend = instance::backend_for_instance(&inst).map_err(|e| e.to_string())?;
    let mut retries = 3;
    loop {
        match backend.destroy().await {
            Ok(_) => { emit("delete_files", "done", "Deleted"); break; }
            Err(e) if retries > 0 => {
                retries -= 1;
                emit("delete_files", "active", &format!("Retrying... ({})", e));
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                // Kill again in case something respawned
                if inst.sandbox_type == SandboxType::Native {
                    instance::kill_native_gateway_public(inst.gateway.gateway_port).await;
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
            Err(e) => {
                emit("delete_files", "error", &e.to_string());
                let _ = app.emit("delete-failed", e.to_string());
                return Err(e.to_string());
            }
        }
    }

    // Stage 4: Update config
    emit("update_config", "active", "Updating config...");
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;
    config.config_mut().instances.retain(|i| i.name != name);
    config.save().map_err(|e| e.to_string())?;
    emit("update_config", "done", "Done");

    let _ = app.emit("delete-complete", ());
    // Single canonical event for state sync. DeleteProgress.tsx already got
    // its own `delete-complete` above; `instance-changed` drives list/tab/
    // health refresh in MainLayout.
    emit_instance_changed(&app, InstanceChanged::deleted(&name));
    Ok(())
}

#[tauri::command]
pub async fn rename_instance(app: tauri::AppHandle, old_name: String, new_name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["rename", &old_name, &new_name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::renamed(&old_name, &new_name));
    Ok(())
}

#[tauri::command]
pub async fn edit_instance_resources(
    app: tauri::AppHandle,
    name: String,
    cpus: Option<u32>,
    memory_mb: Option<u32>,
    disk_gb: Option<u32>,
) -> Result<(), String> {
    let mut args = vec!["edit".to_string(), name.clone()];
    if let Some(c) = cpus { args.extend(["--cpus".into(), c.to_string()]); }
    if let Some(m) = memory_mb { args.extend(["--memory".into(), m.to_string()]); }
    if let Some(d) = disk_gb { args.extend(["--disk".into(), d.to_string()]); }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cli_bridge::run_cli(&refs).await.map_err(|e| e.to_string())?;
    // Changes only take effect after the VM/process restarts — surface that to the user.
    emit_instance_changed(
        &app,
        InstanceChanged::simple(InstanceAction::EditResources, &name).with_needs_restart(true),
    );
    Ok(())
}

#[tauri::command]
pub async fn edit_instance_ports(
    app: tauri::AppHandle,
    name: String,
    gateway_port: u16,
    ttyd_port: u16,
) -> Result<(), String> {
    cli_bridge::run_cli(&[
        "edit", &name,
        "--gateway-port", &gateway_port.to_string(),
        "--ttyd-port", &ttyd_port.to_string(),
    ]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(
        &app,
        InstanceChanged::simple(InstanceAction::EditPorts, &name).with_needs_restart(true),
    );
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
