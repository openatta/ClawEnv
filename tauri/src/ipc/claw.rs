use clawops_core::wire::{ClawTypeInfo, ClawTypesResponse};
use serde::Serialize;

use crate::claw_meta;
use crate::cli_bridge;

/// TypeScript-facing claw-type record. The wire shape (`ClawTypeInfo`)
/// dropped `logo`, `npm_package`, `pip_package` in v2 — we re-derive them
/// here so the existing frontend (`src/types.ts::ClawType`, Home, Install,
/// IconBar) keeps rendering without TS changes.
#[derive(Debug, Clone, Serialize)]
pub struct ClawTypeView {
    pub id: String,
    pub display_name: String,
    /// Emoji or path used by the GUI as the claw's icon. Lives in the
    /// host-side `claw_meta` table, not the wire type.
    pub logo: String,
    pub package_manager: String,
    pub package_id: String,
    /// Legacy split: when the wire `package_manager` is `npm`, this
    /// equals `package_id`; otherwise empty. Kept for the GUI's
    /// `c.npm_package`/`c.pip_package` selector logic.
    pub npm_package: String,
    pub pip_package: String,
    pub default_port: u16,
    pub supports_mcp: bool,
    pub supports_browser: bool,
    pub has_gateway_ui: bool,
    pub supports_native: bool,
}

#[tauri::command]
pub async fn list_claw_types() -> Result<Vec<ClawTypeView>, String> {
    let data = cli_bridge::run_cli(&["claw", "list"]).await.map_err(|e| e.to_string())?;
    let resp: ClawTypesResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp.claw_types.into_iter().map(enrich).collect())
}

fn enrich(t: ClawTypeInfo) -> ClawTypeView {
    let meta = claw_meta::meta_for(&t.id);
    let (npm_package, pip_package) = match t.package_manager.as_str() {
        "npm" => (t.package_id.clone(), String::new()),
        // pip + git_pip both surface their identifier as pip_package
        // for the GUI's binary `npm-or-pip` toggle.
        _ => (String::new(), t.package_id.clone()),
    };
    ClawTypeView {
        id: t.id,
        display_name: t.display_name,
        logo: meta.logo,
        package_manager: t.package_manager,
        package_id: t.package_id,
        npm_package,
        pip_package,
        default_port: t.default_port,
        supports_mcp: t.supports_mcp,
        supports_browser: t.supports_browser,
        has_gateway_ui: t.has_gateway_ui,
        supports_native: t.supports_native,
    }
}
