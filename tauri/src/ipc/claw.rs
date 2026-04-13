use clawenv_core::api::{ClawTypesResponse, ClawTypeInfo};

use crate::cli_bridge;

#[tauri::command]
pub async fn list_claw_types() -> Result<Vec<ClawTypeInfo>, String> {
    let data = cli_bridge::run_cli(&["claw-types"]).await.map_err(|e| e.to_string())?;
    let resp: ClawTypesResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp.claw_types)
}
