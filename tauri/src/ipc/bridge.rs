use clawenv_core::claw::ClawRegistry;
use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;

/// Read the gateway auth token from inside the sandbox
#[tauri::command]
pub async fn get_gateway_token(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let registry = ClawRegistry::load();
    let desc = registry.get(&inst.claw_type);
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    // Try claw-specific config path, then fallback to generic patterns
    let result = backend.exec(&format!(
        "cat ~/.{id}/{id}.json 2>/dev/null | grep -o '\"token\":[ ]*\"[^\"]*\"' | head -1 | sed 's/.*\"\\([^\"]*\\)\"/\\1/'",
        id = desc.id
    )).await.map_err(|e| e.to_string())?;
    Ok(result.trim().to_string())
}

/// Get bridge server configuration
#[tauri::command]
pub fn get_bridge_config() -> Result<serde_json::Value, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let bridge = &config.config().clawenv.bridge;
    serde_json::to_value(bridge).map_err(|e| e.to_string())
}

/// Update bridge server configuration
#[tauri::command]
pub async fn save_bridge_config(bridge_json: String) -> Result<(), String> {
    let bridge: clawenv_core::config::BridgeConfig =
        serde_json::from_str(&bridge_json).map_err(|e| e.to_string())?;
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;
    config.config_mut().clawenv.bridge = bridge;
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_url_in_browser(url: String) -> Result<(), String> {
    // Fallback: use Rust std to open URL
    #[cfg(target_os = "macos")]
    {
        tokio::process::Command::new("open")
            .arg(&url)
            .status()
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        clawenv_core::platform::process::silent_cmd("cmd")
            .args(["/c", "start", &url])
            .status()
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        tokio::process::Command::new("xdg-open")
            .arg(&url)
            .status()
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
