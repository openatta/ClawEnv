use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;

/// Read the gateway auth token from inside the sandbox.
///
/// Tries multiple paths (home dir may vary between users) and uses
/// Node.js JSON parsing instead of fragile grep/sed.
#[tauri::command]
pub async fn get_gateway_token(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let registry = clawenv_core::claw::ClawRegistry::load();
    let desc = registry.get(&inst.claw_type);
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    // Use node to reliably parse JSON; try ~ first, then /home/*/
    let script = format!(
        r#"node -e "
const fs = require('fs'), path = require('path'), id = '{id}';
const candidates = [
  path.join(process.env.HOME || '~', '.'+id, id+'.json'),
  ...require('fs').readdirSync('/home').map(u => '/home/'+u+'/.'+id+'/'+id+'.json').filter(fs.existsSync)
];
for (const f of candidates) {{
  try {{ const j = JSON.parse(fs.readFileSync(f,'utf8')); if (j.token) {{ process.stdout.write(j.token); process.exit(0); }} }} catch {{}}
}}
""#,
        id = desc.id
    );

    let result = backend.exec(&script).await.unwrap_or_default();
    let token = result.trim().to_string();
    if token.is_empty() {
        return Err(format!("No gateway token found for '{name}'. Is the instance running?"));
    }
    Ok(token)
}

/// Get bridge server configuration — direct core (lightweight config read)
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

/// Open URL in platform default browser — GUI-only
#[tauri::command]
pub async fn open_url_in_browser(url: String) -> Result<(), String> {
    clawenv_core::platform::process::open_url(&url).await.map_err(|e| e.to_string())
}
