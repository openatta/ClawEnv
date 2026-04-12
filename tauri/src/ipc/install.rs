use clawenv_core::config::{ConfigManager, UserMode};
use clawenv_core::manager::install;
use clawenv_core::sandbox::InstallMode;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

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
    install_mcp_bridge: Option<bool>,
    gateway_port: u16,
) -> Result<(), String> {
    // Prevent concurrent installs (retry clicks while previous install still running)
    if INSTALL_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Installation already in progress. Please wait for it to finish.".into());
    }

    let mut config = ConfigManager::load()
        .or_else(|_| ConfigManager::create_default(UserMode::General))
        .map_err(|e| { INSTALL_RUNNING.store(false, Ordering::SeqCst); e.to_string() })?;

    // Auto-allocate port: find next free gateway port not used by existing instances
    let actual_port = if gateway_port == 0 {
        let used: Vec<u16> = config.instances().iter().map(|i| i.gateway.gateway_port).collect();
        let mut p = 3000u16;
        while used.contains(&p) { p += 1; }
        p
    } else {
        gateway_port
    };

    let resolved_claw_type = claw_type.unwrap_or_else(|| "openclaw".into());
    let opts = install::InstallOptions {
        instance_name,
        claw_type: resolved_claw_type.clone(),
        claw_version,
        install_mode: InstallMode::OnlineBuild,
        install_browser,
        install_mcp_bridge: install_mcp_bridge.unwrap_or(true),
        api_key,
        use_native,
        gateway_port: actual_port,
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);

    // Spawn a task to forward progress events to the frontend
    let app_handle = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            let _ = app_handle.emit("install-progress", &progress);
        }
    });

    // Spawn the actual installation in the background
    let app_complete = app.clone();
    tokio::spawn(async move {
        match install::install(opts, &mut config, tx).await {
            Ok(()) => {
                let _ = app_complete.emit("install-complete", ());
                crate::tray::send_notification(
                    &app_complete,
                    "Install Complete",
                    &format!("{} has been installed successfully", resolved_claw_type),
                );
            }
            Err(e) => {
                let err_msg = e.to_string();
                let _ = app_complete.emit("install-failed", &err_msg);
                crate::tray::send_notification(
                    &app_complete,
                    "Install Failed",
                    &format!("{} installation failed: {}", resolved_claw_type, err_msg),
                );
            }
        }
        // Release the install lock
        INSTALL_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

/// Install sandbox prerequisites (Lima/Podman/WSL2) if not available
#[tauri::command]
pub async fn install_prerequisites(app: tauri::AppHandle) -> Result<(), String> {
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

/// System check — return detailed system info
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
    /// "info" level means the installer will handle it automatically (show as gray, not red)
    #[serde(default)]
    pub info_only: bool,
}

#[tauri::command]
pub async fn system_check() -> Result<SystemCheckInfo, String> {
    use clawenv_core::platform::detect_platform;
    use clawenv_core::sandbox::detect_backend;

    let platform = detect_platform().map_err(|e| e.to_string())?;

    let os_str = format!("{:?}", platform.os);
    let arch_str = format!("{:?}", platform.arch);

    // Memory detection (cross-platform)
    let memory_gb = clawenv_core::platform::process::system_memory_gb().await;

    // Disk free space (cross-platform)
    let disk_free_gb = clawenv_core::platform::process::disk_free_gb().await;

    // Sandbox backend
    let (backend_name, backend_available) = match detect_backend() {
        Ok(b) => {
            let available = b.is_available().await.unwrap_or(false);
            (b.name().to_string(), available)
        }
        Err(e) => (format!("Error: {e}"), false),
    };

    // Build check items
    let mut checks = vec![];

    // OS check
    checks.push(CheckItem {
        name: "Operating System".into(),
        ok: true,
        detail: format!("{} ({})", os_str, arch_str),
        info_only: false,
    });

    // Memory check (OpenClaw needs at least 512MB for sandbox)
    let mem_ok = memory_gb >= 2.0;
    checks.push(CheckItem {
        name: "Memory".into(),
        ok: mem_ok,
        detail: format!("{:.1} GB {}", memory_gb, if mem_ok { "(sufficient)" } else { "(need 2GB+)" }),
        info_only: false,
    });

    // Disk check (need at least 2GB free)
    let disk_ok = disk_free_gb >= 2.0;
    checks.push(CheckItem {
        name: "Disk Space".into(),
        ok: disk_ok,
        detail: format!("{:.0} GB free {}", disk_free_gb, if disk_ok { "(sufficient)" } else { "(need 2GB+)" }),
        info_only: false,
    });

    // Sandbox backend — if not installed, it's info-only (installer will auto-install)
    checks.push(CheckItem {
        name: "Sandbox Backend".into(),
        ok: backend_available,
        detail: format!("{} {}", backend_name, if backend_available { "(ready)" } else { "(will be installed automatically)" }),
        info_only: !backend_available,
    });

    Ok(SystemCheckInfo {
        os: os_str,
        os_version: String::new(),
        arch: arch_str,
        memory_gb,
        disk_free_gb,
        sandbox_backend: backend_name,
        sandbox_available: backend_available,
        checks,
    })
}

/// Test API key by making a request to OpenClaw API
#[tauri::command]
pub async fn test_api_key(api_key: String) -> Result<String, String> {
    if api_key.is_empty() {
        return Err("API key is empty".into());
    }
    if !api_key.starts_with("sk-") {
        return Err("API key should start with 'sk-'".into());
    }
    // Basic format validation passed
    // In real implementation, this would call the OpenClaw API to verify
    Ok("API key format valid".into())
}

/// Restart the computer (for WSL2 installation completion)
#[tauri::command]
pub async fn restart_computer() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        clawenv_core::platform::process::silent_cmd("shutdown")
            .args(["/r", "/t", "5", "/c", "ClawEnv: Restarting to complete WSL2 installation"])
            .status()
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        return Err("Restart is only needed on Windows for WSL2 installation".into());
    }
    Ok(())
}
