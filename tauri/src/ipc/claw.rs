use clawenv_core::claw::ClawRegistry;
use serde::Serialize;

#[derive(Serialize)]
pub struct ClawTypeInfo {
    pub id: String,
    pub display_name: String,
    pub logo: String,
    pub npm_package: String,
    pub default_port: u16,
    pub supports_mcp: bool,
    pub supports_browser: bool,
}

#[tauri::command]
pub fn list_claw_types() -> Vec<ClawTypeInfo> {
    let registry = ClawRegistry::load();
    registry.list_all().iter().map(|d| ClawTypeInfo {
        id: d.id.clone(),
        display_name: d.display_name.clone(),
        logo: d.logo.clone(),
        npm_package: d.npm_package.clone(),
        default_port: d.default_port,
        supports_mcp: d.supports_mcp,
        supports_browser: d.supports_browser,
    }).collect()
}
