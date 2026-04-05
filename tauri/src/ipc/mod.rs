use clawenv_core::config::{ConfigManager, ProxyConfig, UserMode};
use clawenv_core::launcher::{self, LaunchState};
use clawenv_core::manager::{install, instance};
use clawenv_core::sandbox::InstallMode;
use serde::Serialize;
use tauri::Emitter;

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
        })
        .collect())
}

#[tauri::command]
pub async fn install_openclaw(
    app: tauri::AppHandle,
    instance_name: String,
    claw_version: String,
    api_key: Option<String>,
    use_native: bool,
    install_browser: bool,
    gateway_port: u16,
) -> Result<(), String> {
    let opts = install::InstallOptions {
        instance_name,
        claw_version,
        install_mode: InstallMode::OnlineBuild,
        install_browser,
        api_key,
        use_native,
        gateway_port,
    };

    let mut config = ConfigManager::load()
        .or_else(|_| ConfigManager::create_default(UserMode::General))
        .map_err(|e| e.to_string())?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);

    // Spawn a task to forward progress events to the frontend
    let app_handle = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            let _ = app_handle.emit("install-progress", &progress);
        }
    });

    // Spawn the actual installation in the background
    let app_complete = app.clone();
    tokio::spawn(async move {
        match install::install(opts, &mut config, tx).await {
            Ok(()) => {
                let _ = app_complete.emit("install-complete", ());
                crate::tray::send_notification(
                    &app_complete,
                    "Install Complete",
                    "OpenClaw has been installed successfully",
                );
            }
            Err(e) => {
                let err_msg = e.to_string();
                let _ = app_complete.emit("install-failed", &err_msg);
                crate::tray::send_notification(
                    &app_complete,
                    "Install Failed",
                    &format!("OpenClaw installation failed: {}", err_msg),
                );
            }
        }
    });

    Ok(())
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
pub async fn get_instance_health(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let health = instance::instance_health(inst).await;
    Ok(format!("{:?}", health))
}

#[tauri::command]
pub async fn save_settings(settings_json: String) -> Result<(), String> {
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;

    // Parse the incoming JSON as partial config fields
    let values: serde_json::Value =
        serde_json::from_str(&settings_json).map_err(|e| e.to_string())?;

    let cfg = config.config_mut();

    if let Some(lang) = values.get("language").and_then(|v| v.as_str()) {
        cfg.clawenv.language = lang.to_string();
    }
    if let Some(theme) = values.get("theme").and_then(|v| v.as_str()) {
        cfg.clawenv.theme = theme.to_string();
    }
    if let Some(proxy) = values.get("proxy") {
        if let Ok(p) = serde_json::from_value::<ProxyConfig>(proxy.clone()) {
            // Store proxy password in keychain if present in JSON
            if let Some(password) = proxy.get("auth_password").and_then(|v| v.as_str()) {
                if !password.is_empty() {
                    let _ = clawenv_core::config::keychain::store_proxy_password(password);
                }
            }
            cfg.clawenv.proxy = p;
        }
    }
    if let Some(auto_check) = values.get("auto_check_updates").and_then(|v| v.as_bool()) {
        cfg.clawenv.updates.auto_check = auto_check;
    }

    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_proxy(proxy_json: String) -> Result<(), String> {
    let proxy: ProxyConfig =
        serde_json::from_str(&proxy_json).map_err(|e| e.to_string())?;
    clawenv_core::config::proxy::test_proxy(&proxy)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_default_config(user_mode: String) -> Result<(), String> {
    let mode = match user_mode.to_lowercase().as_str() {
        "developer" | "dev" => UserMode::Developer,
        _ => UserMode::General,
    };
    ConfigManager::create_default(mode).map_err(|e| e.to_string())?;
    Ok(())
}
