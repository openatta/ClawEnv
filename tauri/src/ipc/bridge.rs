use clawops_core::bridge::read_gateway_token;
use clawops_core::config_loader;
use clawops_core::instance::InstanceRegistry;

use crate::instance_helper;
use crate::util;

/// Read the gateway auth token from inside the sandbox.
///
/// v2 lifted the JSON-parse + multi-path-probe logic into
/// `clawops_core::bridge::read_gateway_token`, so the Tauri side just
/// resolves the instance + sandbox backend and forwards.
#[tauri::command]
pub async fn get_gateway_token(name: String) -> Result<String, String> {
    let registry = InstanceRegistry::with_default_path();
    let inst = registry.find(&name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Instance '{name}' not found"))?;
    let backend = instance_helper::backend_arc_for_instance(&inst)?;
    let token = read_gateway_token(&backend, &inst.claw)
        .await
        .map_err(|e| e.to_string())?;
    if token.is_empty() {
        return Err(format!("No gateway token found for '{name}'. Is the instance running?"));
    }
    Ok(token)
}

#[tauri::command]
pub fn get_bridge_config() -> Result<serde_json::Value, String> {
    let global = config_loader::load_global().map_err(|e| e.to_string())?;
    serde_json::to_value(&global.bridge).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_bridge_config(bridge_json: String) -> Result<(), String> {
    let bridge: config_loader::BridgeConfig =
        serde_json::from_str(&bridge_json).map_err(|e| e.to_string())?;
    config_loader::save_bridge_section(&bridge).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_url_in_browser(url: String) -> Result<(), String> {
    util::open_url(&url).await.map_err(|e| e.to_string())
}
