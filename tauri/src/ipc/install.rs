use clawenv_core::api::SystemCheckResponse;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use crate::cli_bridge::{self, CliEvent};

/// Guard against concurrent installs — only one install at a time.
static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub async fn install_openclaw(
    app: tauri::AppHandle,
    instance_name: String,
    claw_type: Option<String>,
    claw_version: String,
    api_key: Option<String>,
    use_native: bool,
    install_browser: bool,
    _install_mcp_bridge: Option<bool>,
    gateway_port: u16,
) -> Result<(), String> {
    if INSTALL_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Installation already in progress. Please wait for it to finish.".into());
    }

    let ct = claw_type.unwrap_or_else(|| "openclaw".into());
    let mode = if use_native { "native" } else { "sandbox" };

    // Build CLI args
    let mut args = vec![
        "install".to_string(),
        "--mode".to_string(), mode.to_string(),
        "--claw-type".to_string(), ct.clone(),
        "--version".to_string(), claw_version,
        "--name".to_string(), instance_name,
        "--port".to_string(), gateway_port.to_string(),
    ];
    if install_browser {
        args.push("--browser".to_string());
    }
    if let Some(key) = api_key {
        args.push("--api-key".to_string());
        args.push(key);
    }

    let app_handle = app.clone();
    tokio::spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<CliEvent>(32);

        // Forward CLI events to Tauri frontend
        let app_fwd = app_handle.clone();
        let fwd_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match &event {
                    CliEvent::Progress { stage, percent, message } => {
                        let _ = app_fwd.emit("install-progress", serde_json::json!({
                            "stage": stage,
                            "percent": percent,
                            "message": message,
                        }));
                    }
                    CliEvent::Info { message } => {
                        let _ = app_fwd.emit("install-progress", serde_json::json!({
                            "stage": "Info",
                            "percent": 0,
                            "message": message,
                        }));
                    }
                    _ => {}
                }
            }
        });

        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let result = cli_bridge::run_cli_streaming(&args_ref, tx).await;
        fwd_task.await.ok();

        match result {
            Ok(_) => {
                let _ = app_handle.emit("install-complete", ());
                crate::tray::send_notification(
                    &app_handle,
                    "Install Complete",
                    &format!("{ct} has been installed successfully"),
                );
            }
            Err(e) => {
                let err_msg = e.to_string();
                let _ = app_handle.emit("install-failed", &err_msg);
                crate::tray::send_notification(
                    &app_handle,
                    "Install Failed",
                    &format!("{ct} installation failed: {err_msg}"),
                );
            }
        }
        INSTALL_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

#[tauri::command]
pub async fn install_prerequisites(app: tauri::AppHandle) -> Result<(), String> {
    // Prerequisites install is not in CLI yet — keep direct core call
    use clawenv_core::sandbox::detect_backend;

    let _ = app.emit("prereq-step", "Detecting sandbox backend...");
    let backend = detect_backend().map_err(|e| e.to_string())?;

    let available = backend.is_available().await.unwrap_or(false);
    if available {
        let _ = app.emit("prereq-step", &format!("{} is already installed", backend.name()));
        return Ok(());
    }

    let _ = app.emit("prereq-step", &format!("{} not found, installing...", backend.name()));
    backend.ensure_prerequisites().await.map_err(|e| e.to_string())?;
    let _ = app.emit("prereq-step", &format!("{} installed successfully", backend.name()));

    Ok(())
}

#[derive(Serialize)]
pub struct SystemCheckInfo {
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub memory_gb: f64,
    pub disk_free_gb: f64,
    pub sandbox_backend: String,
    pub sandbox_available: bool,
    pub checks: Vec<CheckItem>,
}

#[derive(Serialize)]
pub struct CheckItem {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    #[serde(default)]
    pub info_only: bool,
}

#[tauri::command]
pub async fn system_check() -> Result<SystemCheckInfo, String> {
    let data = cli_bridge::run_cli(&["system-check"]).await.map_err(|e| e.to_string())?;
    let resp: SystemCheckResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;

    Ok(SystemCheckInfo {
        os: resp.os,
        os_version: String::new(),
        arch: resp.arch,
        memory_gb: resp.memory_gb,
        disk_free_gb: resp.disk_free_gb,
        sandbox_backend: resp.sandbox_backend.clone(),
        sandbox_available: resp.sandbox_available,
        checks: resp.checks.into_iter().map(|c| CheckItem {
            name: c.name,
            ok: c.ok,
            detail: c.detail,
            info_only: c.info_only,
        }).collect(),
    })
}

#[tauri::command]
pub async fn test_api_key(api_key: String) -> Result<String, String> {
    if api_key.is_empty() {
        return Err("API key is empty".into());
    }
    if !api_key.starts_with("sk-") {
        return Err("API key should start with 'sk-'".into());
    }
    Ok("API key format valid".into())
}

#[tauri::command]
pub async fn restart_computer() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        clawenv_core::platform::process::silent_cmd("shutdown")
            .args(["/r", "/t", "5", "/c", "ClawEnv: Restarting to complete WSL2 installation"])
            .status()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Restart is only needed on Windows for WSL2 installation".into())
    }
}
