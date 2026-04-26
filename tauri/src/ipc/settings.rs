//! Settings page handlers — drive `[clawenv.proxy]`, `[clawenv.bridge]`, and
//! a small set of scalar fields under `[clawenv]` (language, theme).
//!
//! v2 `config_loader` is function-based rather than a stateful
//! `ConfigManager`, so each save lands a focused write through
//! `save_clawenv_field` / `save_proxy_section`. This is also
//! transactionally cleaner than the v1 "load → mutate → save the whole
//! file" pattern, which round-tripped any unrelated edits the user made
//! out-of-band.

use clawops_core::config_loader;
use clawops_core::credentials;
use clawops_core::instance::{InstanceRegistry, SandboxKind};
use clawops_core::proxy::ProxyConfig;

#[tauri::command]
pub async fn save_settings(settings_json: String) -> Result<(), String> {
    let values: serde_json::Value =
        serde_json::from_str(&settings_json).map_err(|e| e.to_string())?;

    if let Some(lang) = values.get("language").and_then(|v| v.as_str()) {
        config_loader::save_clawenv_field("language", lang)
            .map_err(|e| e.to_string())?;
    }
    if let Some(theme) = values.get("theme").and_then(|v| v.as_str()) {
        config_loader::save_clawenv_field("theme", theme)
            .map_err(|e| e.to_string())?;
    }
    if let Some(proxy) = values.get("proxy") {
        if let Ok(p) = serde_json::from_value::<ProxyConfig>(proxy.clone()) {
            if let Some(password) = proxy.get("auth_password").and_then(|v| v.as_str()) {
                if !password.is_empty() {
                    let _ = credentials::store_proxy_password(password);
                }
            }
            config_loader::save_proxy_section(&p).map_err(|e| e.to_string())?;
        }
    }
    if let Some(auto_check) = values.get("auto_check_updates").and_then(|v| v.as_bool()) {
        config_loader::save_clawenv_field("auto_check_updates", &auto_check.to_string())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn autostart_is_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn autostart_set(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    if enabled {
        app.autolaunch().enable().map_err(|e| e.to_string())
    } else {
        app.autolaunch().disable().map_err(|e| e.to_string())
    }
}

/// First-run config bootstrap. v2's `config_loader::load_global` returns
/// defaults for a missing file, so the only "creation" step is writing
/// the `user_mode` field so subsequent loads echo it back.
#[tauri::command]
pub async fn create_default_config(user_mode: String) -> Result<(), String> {
    let mode = match user_mode.to_lowercase().as_str() {
        "developer" | "dev" => "developer",
        _ => "general",
    };
    config_loader::save_clawenv_field("user_mode", mode).map_err(|e| e.to_string())
}

/// Diagnose instance / on-disk consistency. Returns issues + offers repair.
#[tauri::command]
pub async fn diagnose_instances() -> Result<serde_json::Value, String> {
    let registry = InstanceRegistry::with_default_path();
    let instances = registry.list().await.map_err(|e| e.to_string())?;
    let home = dirs::home_dir().unwrap_or_default();
    let mut issues: Vec<serde_json::Value> = Vec::new();

    for inst in &instances {
        match inst.backend {
            SandboxKind::Native => {
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
            SandboxKind::Lima => {
                let vm_dir = clawops_core::paths::lima_home().join(&inst.sandbox_instance);
                if !vm_dir.exists() {
                    issues.push(serde_json::json!({
                        "instance": inst.name, "type": "missing_vm",
                        "message": format!("Lima VM missing: {}", inst.sandbox_instance),
                        "fixable": true,
                    }));
                }
            }
            _ => {}
        }
    }

    let native_dir = home.join(".clawenv").join("native");
    if native_dir.exists() && !instances.iter().any(|i| i.backend == SandboxKind::Native) {
        issues.push(serde_json::json!({
            "instance": "(orphan)", "type": "orphan_native",
            "message": "Orphan native directory exists but no native instance in registry",
            "fixable": true,
        }));
    }

    // Legacy Lima VMs left in the system default ~/.lima/ from old builds
    // (before LIMA_HOME was pinned to ~/.clawenv/lima).
    let legacy_lima = home.join(".lima");
    if legacy_lima.exists() {
        if let Ok(mut rd) = std::fs::read_dir(&legacy_lima) {
            let has_clawenv_vm = rd.any(|e| e.ok()
                .and_then(|e| e.file_name().into_string().ok())
                .map(|n| n.starts_with("clawenv-"))
                .unwrap_or(false));
            if has_clawenv_vm {
                issues.push(serde_json::json!({
                    "instance": "(legacy)", "type": "legacy_lima_home",
                    "message": "Legacy Lima VMs found in ~/.lima/. clawenv now stores VMs under ~/.clawenv/lima/. Move or delete the old directory.",
                    "fixable": false,
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "issues": issues,
        "instance_count": instances.len(),
    }))
}

#[tauri::command]
pub async fn fix_diagnostic_issue(instance_name: String, issue_type: String) -> Result<(), String> {
    let home = dirs::home_dir().unwrap_or_default();

    match issue_type.as_str() {
        "missing_dir" | "missing_vm" => {
            let registry = InstanceRegistry::with_default_path();
            let _ = registry.remove(&instance_name).await;
        }
        "orphan_native" => {
            let native_dir = home.join(".clawenv").join("native");
            tokio::fs::remove_dir_all(&native_dir).await.ok();
        }
        _ => return Err(format!("Unknown issue type: {issue_type}")),
    }
    Ok(())
}
