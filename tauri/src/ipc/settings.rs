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

#[tauri::command]
pub async fn create_default_config(user_mode: String) -> Result<(), String> {
    let mode = match user_mode.to_lowercase().as_str() {
        "developer" | "dev" => UserMode::Developer,
        _ => UserMode::General,
    };
    ConfigManager::create_default(mode).map_err(|e| e.to_string())?;
    Ok(())
}
