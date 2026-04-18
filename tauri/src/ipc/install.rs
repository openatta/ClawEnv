use clawenv_core::api::{SystemCheckResponse, CheckItem as ApiCheckItem};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use crate::cli_bridge::{self, CliEvent};
use crate::ipc::emit::{emit_instance_changed, InstanceAction, InstanceChanged};

/// Guard against concurrent installs — only one install at a time.
static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);

/// RAII guard that resets `INSTALL_RUNNING` on drop. A previous iteration
/// set the flag on entry and cleared it at the end of the happy path — if
/// any code in the async task panicked (tokio catches panics but the
/// trailing `store(false)` still got skipped), the flag stayed `true`
/// forever, leaving the user permanently blocked from starting another
/// install. A drop guard runs no matter how the scope exits.
struct InstallRunningGuard;
impl Drop for InstallRunningGuard {
    fn drop(&mut self) {
        INSTALL_RUNNING.store(false, Ordering::SeqCst);
    }
}

// This is a Tauri `#[command]` IPC endpoint — its argument list is the wire
// protocol between the install wizard frontend and the backend. Packing
// the fields into a struct just forces every JS caller to build an object
// with the same keys, adding indirection without simplification.
#[allow(clippy::too_many_arguments)]
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
    image: Option<String>,
) -> Result<(), String> {
    if INSTALL_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Installation already in progress. Please wait for it to finish.".into());
    }

    let ct = claw_type.unwrap_or_else(|| "openclaw".into());
    let mode = if use_native { "native" } else { "sandbox" };
    // Keep the instance name around for the post-install instance-changed
    // emit — `instance_name` itself is moved into the CLI args vec below.
    let instance_name_for_emit = instance_name.clone();

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
    if let Some(ref img) = image {
        if !img.is_empty() {
            args.push("--image".to_string());
            args.push(img.clone());
        }
    }

    let app_handle = app.clone();
    tokio::spawn(async move {
        // Guard released when this task exits, via any path (normal return,
        // early return, panic caught by tokio's spawn, task cancel).
        let _guard = InstallRunningGuard;
        let (tx, mut rx) = tokio::sync::mpsc::channel::<CliEvent>(32);

        // Forward CLI events to Tauri frontend
        let app_fwd = app_handle.clone();
        let fwd_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match &event {
                    CliEvent::Progress { .. } | CliEvent::Info { .. } => {
                        // Forward as structured event (Serialize derives available)
                        let _ = app_fwd.emit("install-progress", &event);
                    }
                    CliEvent::Complete { .. } => {
                        let _ = app_fwd.emit("install-progress", &event);
                    }
                    CliEvent::Error { .. } => {
                        let _ = app_fwd.emit("install-progress", &event);
                    }
                    CliEvent::Data { .. } => {
                        let _ = app_fwd.emit("install-progress", &event);
                    }
                }
            }
        });

        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let result = cli_bridge::run_cli_streaming(&args_ref, tx, |_| {}).await;
        fwd_task.await.ok();

        match result {
            Ok(_) => {
                let _ = app_handle.emit("install-complete", ());
                // Canonical state-sync event. The install runs in an isolated
                // WebviewWindow, so the main window doesn't inspect config.toml
                // on its own — MainLayout's `instance-changed` listener is the
                // single code path that refreshes `instances()` and makes the
                // newly-installed entry appear in Home / ClawPage. The separate
                // front-end emit in App.tsx on window close is belt-and-braces;
                // this backend emit is the authoritative one that can't be
                // accidentally skipped if the install window is force-closed.
                emit_instance_changed(
                    &app_handle,
                    InstanceChanged::simple(InstanceAction::Install, &instance_name_for_emit),
                );
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
        // _guard drops here and resets INSTALL_RUNNING — no manual store needed.
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
    pub arch: String,
    pub memory_gb: f64,
    pub disk_free_gb: f64,
    pub sandbox_backend: String,
    pub sandbox_available: bool,
    pub checks: Vec<ApiCheckItem>,
}

#[tauri::command]
pub async fn system_check() -> Result<SystemCheckInfo, String> {
    let data = cli_bridge::run_cli(&["system-check"]).await.map_err(|e| e.to_string())?;
    let resp: SystemCheckResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;

    Ok(SystemCheckInfo {
        os: resp.os,
        arch: resp.arch,
        memory_gb: resp.memory_gb,
        disk_free_gb: resp.disk_free_gb,
        sandbox_backend: resp.sandbox_backend.clone(),
        sandbox_available: resp.sandbox_available,
        checks: resp.checks,
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

/// Open file picker for import and return selected path
#[tauri::command]
pub async fn pick_import_file(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app.dialog().file()
        .add_filter("ClawEnv Package", &["tar.gz", "gz"])
        .blocking_pick_file();
    match path {
        Some(p) => Ok(p.to_string()),
        None => Err("No file selected".into()),
    }
}

/// Validate an import file name against current platform
#[tauri::command]
pub async fn validate_import_file(file_path: String) -> Result<serde_json::Value, String> {
    let filename = std::path::Path::new(&file_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    // Expected format: {platform}-{arch}-{timestamp}.tar.gz
    let parts: Vec<&str> = filename.split('-').collect();
    if parts.len() < 3 {
        return Ok(serde_json::json!({
            "valid": false,
            "error": "Unrecognized file name format. Expected: {platform}-{arch}-{timestamp}.tar.gz",
            "is_native": false,
        }));
    }

    let file_platform = parts[0];
    let file_arch = parts[1];

    // Determine if native or sandbox
    let is_native = matches!(file_platform, "windows" | "macos" | "linux");
    let is_sandbox = matches!(file_platform, "lima" | "wsl2" | "podman");

    if !is_native && !is_sandbox {
        return Ok(serde_json::json!({
            "valid": false,
            "error": format!("Unknown platform '{}' in file name", file_platform),
            "is_native": false,
        }));
    }

    // Check platform match
    let current_platform = if cfg!(target_os = "macos") { "macos" }
        else if cfg!(target_os = "windows") { "windows" }
        else { "linux" };
    let current_backend = if cfg!(target_os = "macos") { "lima" }
        else if cfg!(target_os = "windows") { "wsl2" }
        else { "podman" };

    let platform_ok = if is_native {
        file_platform == current_platform
    } else {
        file_platform == current_backend
    };

    // Check arch match
    let current_arch = std::env::consts::ARCH;
    let arch_ok = file_arch == current_arch
        || (file_arch == "arm64" && current_arch == "aarch64")
        || (file_arch == "aarch64" && current_arch == "aarch64")
        || (file_arch == "x64" && current_arch == "x86_64")
        || (file_arch == "x86_64" && current_arch == "x86_64");

    let mut errors = Vec::new();
    if !platform_ok {
        errors.push(format!("Platform mismatch: file is for '{}', this machine is '{}'",
            file_platform, if is_native { current_platform } else { current_backend }));
    }
    if !arch_ok {
        errors.push(format!("Architecture mismatch: file is for '{}', this machine is '{}'",
            file_arch, current_arch));
    }

    Ok(serde_json::json!({
        "valid": errors.is_empty(),
        "error": errors.join("; "),
        "is_native": is_native,
        "platform": file_platform,
        "arch": file_arch,
    }))
}

/// Check if a native instance already exists
#[tauri::command]
pub async fn has_native_instance() -> Result<bool, String> {
    use clawenv_core::config::ConfigManager;
    use clawenv_core::sandbox::SandboxType;
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    Ok(config.instances().iter().any(|i| i.sandbox_type == SandboxType::Native))
}
