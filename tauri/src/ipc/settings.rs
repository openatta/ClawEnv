// Settings use direct core calls — GUI settings page submits multiple fields
// at once, which is more efficient as a single config load/save cycle.
use clawenv_core::config::{ConfigManager, ProxyConfig, UserMode};

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

/// Check if autostart is enabled at OS level
#[tauri::command]
pub async fn autostart_is_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Enable or disable autostart at OS level
#[tauri::command]
pub async fn autostart_set(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    if enabled {
        app.autolaunch().enable().map_err(|e| e.to_string())
    } else {
        app.autolaunch().disable().map_err(|e| e.to_string())
    }
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

/// Diagnose instance/config consistency. Returns issues and offers repair.
#[tauri::command]
pub async fn diagnose_instances() -> Result<serde_json::Value, String> {
    use clawenv_core::sandbox::SandboxType;

    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let home = dirs::home_dir().unwrap_or_default();
    let mut issues: Vec<serde_json::Value> = Vec::new();

    for inst in config.instances() {
        match inst.sandbox_type {
            SandboxType::Native => {
                let native_dir = home.join(".clawenv").join("native");
                if !native_dir.exists() {
                    issues.push(serde_json::json!({
                        "instance": inst.name, "type": "missing_dir",
                        "message": format!("Native directory missing: {}", native_dir.display()),
                        "fixable": true,
                    }));
                }
                let node_dir = home.join(".clawenv").join("node");
                if !node_dir.exists() {
                    issues.push(serde_json::json!({
                        "instance": inst.name, "type": "missing_node",
                        "message": "ClawEnv Node.js not installed (~/.clawenv/node/)",
                        "fixable": false,
                    }));
                }
            }
            SandboxType::LimaAlpine => {
                let vm_dir = home.join(".lima").join(&inst.sandbox_id);
                if !vm_dir.exists() {
                    issues.push(serde_json::json!({
                        "instance": inst.name, "type": "missing_vm",
                        "message": format!("Lima VM missing: {}", inst.sandbox_id),
                        "fixable": true,
                    }));
                }
            }
            _ => {}
        }
    }

    // Check orphan directories
    let native_dir = home.join(".clawenv").join("native");
    if native_dir.exists() && !config.instances().iter().any(|i| i.sandbox_type == SandboxType::Native) {
        issues.push(serde_json::json!({
            "instance": "(orphan)", "type": "orphan_native",
            "message": "Orphan native directory exists but no native instance in config",
            "fixable": true,
        }));
    }

    Ok(serde_json::json!({
        "issues": issues,
        "instance_count": config.instances().len(),
    }))
}

/// Fix a diagnostic issue by removing orphan data or config entries
#[tauri::command]
pub async fn fix_diagnostic_issue(instance_name: String, issue_type: String) -> Result<(), String> {
    let home = dirs::home_dir().unwrap_or_default();

    match issue_type.as_str() {
        "missing_dir" | "missing_vm" => {
            // Remove instance from config (data is gone)
            let mut config = ConfigManager::load().map_err(|e| e.to_string())?;
            config.config_mut().instances.retain(|i| i.name != instance_name);
            config.save().map_err(|e| e.to_string())?;
        }
        "orphan_native" => {
            // Delete orphan native directory
            let native_dir = home.join(".clawenv").join("native");
            tokio::fs::remove_dir_all(&native_dir).await.ok();
        }
        _ => return Err(format!("Unknown issue type: {issue_type}")),
    }
    Ok(())
}
