use clawenv_core::config::{ConfigManager, UserMode};
use clawenv_core::manager::install;
use clawenv_core::sandbox::InstallMode;
use serde::Serialize;
use tauri::Emitter;

#[tauri::command]
pub async fn install_openclaw(
    app: tauri::AppHandle,
    instance_name: String,
    claw_version: String,
    api_key: Option<String>,
    use_native: bool,
    install_browser: bool,
    install_mcp_bridge: Option<bool>,
    gateway_port: u16,
) -> Result<(), String> {
    let opts = install::InstallOptions {
        instance_name,
        claw_version,
        install_mode: InstallMode::OnlineBuild,
        install_browser,
        install_mcp_bridge: install_mcp_bridge.unwrap_or(true),
        api_key,
        use_native,
        gateway_port,
    };

    let mut config = ConfigManager::load()
        .or_else(|_| ConfigManager::create_default(UserMode::General))
        .map_err(|e| e.to_string())?;

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
                    "OpenClaw has been installed successfully",
                );
            }
            Err(e) => {
                let err_msg = e.to_string();
                let _ = app_complete.emit("install-failed", &err_msg);
                crate::tray::send_notification(
                    &app_complete,
                    "Install Failed",
                    &format!("OpenClaw installation failed: {}", err_msg),
                );
            }
        }
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
}

#[tauri::command]
pub async fn system_check() -> Result<SystemCheckInfo, String> {
    use clawenv_core::platform::detect_platform;
    use clawenv_core::sandbox::detect_backend;

    let platform = detect_platform().map_err(|e| e.to_string())?;

    let os_str = format!("{:?}", platform.os);
    let arch_str = format!("{:?}", platform.arch);

    // Memory detection (cross-platform)
    let memory_gb = {
        #[cfg(target_os = "macos")]
        {
            let out = tokio::process::Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output().await;
            out.ok().and_then(|o| {
                String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()
            }).unwrap_or(0.0) / 1_073_741_824.0
        }
        #[cfg(target_os = "linux")]
        {
            // /proc/meminfo: MemTotal: 16384000 kB
            let out = tokio::fs::read_to_string("/proc/meminfo").await;
            out.ok().and_then(|s| {
                s.lines().find(|l| l.starts_with("MemTotal"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<f64>().ok())
            }).unwrap_or(0.0) / 1_048_576.0 // kB to GB
        }
        #[cfg(target_os = "windows")]
        {
            let out = tokio::process::Command::new("wmic")
                .args(["ComputerSystem", "get", "TotalPhysicalMemory", "/value"])
                .output().await;
            out.ok().and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines().find(|l| l.contains("="))
                    .and_then(|l| l.split('=').nth(1))
                    .and_then(|v| v.trim().parse::<f64>().ok())
            }).unwrap_or(0.0) / 1_073_741_824.0
        }
    };

    // Disk free space (cross-platform)
    let disk_free_gb = {
        #[cfg(target_os = "macos")]
        {
            let out = tokio::process::Command::new("df")
                .args(["-g", "/"])
                .output().await;
            out.ok().and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines().nth(1)
                    .and_then(|l| l.split_whitespace().nth(3))
                    .and_then(|v| v.parse::<f64>().ok())
            }).unwrap_or(0.0)
        }
        #[cfg(target_os = "linux")]
        {
            let out = tokio::process::Command::new("df")
                .args(["--output=avail", "-BG", "/"])
                .output().await;
            out.ok().and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines().nth(1)
                    .and_then(|l| l.trim().trim_end_matches('G').parse::<f64>().ok())
            }).unwrap_or(0.0)
        }
        #[cfg(target_os = "windows")]
        {
            let out = tokio::process::Command::new("wmic")
                .args(["LogicalDisk", "where", "DeviceID='C:'", "get", "FreeSpace", "/value"])
                .output().await;
            out.ok().and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines().find(|l| l.contains("="))
                    .and_then(|l| l.split('=').nth(1))
                    .and_then(|v| v.trim().parse::<f64>().ok())
            }).unwrap_or(0.0) / 1_073_741_824.0
        }
    };

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
    });

    // Memory check (OpenClaw needs at least 512MB for sandbox)
    let mem_ok = memory_gb >= 2.0;
    checks.push(CheckItem {
        name: "Memory".into(),
        ok: mem_ok,
        detail: format!("{:.1} GB {}", memory_gb, if mem_ok { "(sufficient)" } else { "(need 2GB+)" }),
    });

    // Disk check (need at least 2GB free)
    let disk_ok = disk_free_gb >= 2.0;
    checks.push(CheckItem {
        name: "Disk Space".into(),
        ok: disk_ok,
        detail: format!("{:.0} GB free {}", disk_free_gb, if disk_ok { "(sufficient)" } else { "(need 2GB+)" }),
    });

    // Sandbox backend
    checks.push(CheckItem {
        name: "Sandbox Backend".into(),
        ok: backend_available,
        detail: format!("{} {}", backend_name, if backend_available { "(ready)" } else { "(not installed)" }),
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
