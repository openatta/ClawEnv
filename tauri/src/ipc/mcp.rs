//! IPC commands for the local MCP server's permission UX.
//!
//! macOS users have to grant Accessibility (synthetic kbd/mouse) and
//! Screen Recording before any input/screen tool can run. The GUI's
//! settings page calls these to show the user the current state and
//! deep-link them into System Settings.

use clawops_core::input::perm;
use serde::Serialize;

#[derive(Serialize)]
pub struct McpPermStatus {
    pub accessibility: String,
    pub screen_capture: String,
    pub all_granted: bool,
    /// Bilingual prompt to show when something is missing. None when
    /// everything is granted (and on Windows where there's nothing to
    /// gate). Caller decides whether to render as a toast / dialog.
    pub guidance_headline: Option<String>,
    pub guidance_steps: Vec<String>,
    pub guidance_open_urls: Vec<String>,
}

#[tauri::command]
pub async fn mcp_perm_status() -> Result<McpPermStatus, String> {
    let report = perm::probe();
    let g = perm::guidance_for(&report);
    Ok(McpPermStatus {
        accessibility: report.accessibility.as_str().into(),
        screen_capture: report.screen_capture.as_str().into(),
        all_granted: report.all_granted(),
        guidance_headline: g.as_ref().map(|m| m.headline.clone()),
        guidance_steps: g.as_ref().map(|m| m.steps.clone()).unwrap_or_default(),
        guidance_open_urls: g.map(|m| m.open_urls).unwrap_or_default(),
    })
}

/// Deep-link into the relevant System Settings pane. macOS only; on
/// Windows we no-op. The URL strings come from `perm::guidance_for`.
#[tauri::command]
pub async fn mcp_open_perm_url(url: String) -> Result<(), String> {
    crate::util::open_url(&url).await.map_err(|e| e.to_string())
}
