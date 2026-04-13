use clawenv_core::api::SandboxListResponse;
use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
#[cfg(target_os = "windows")]
use clawenv_core::platform::process::silent_cmd;
use tauri::Emitter;

use crate::cli_bridge;

#[tauri::command]
pub async fn list_sandbox_vms() -> Result<Vec<clawenv_core::api::SandboxVmInfo>, String> {
    let data = cli_bridge::run_cli(&["sandbox", "list"]).await.map_err(|e| e.to_string())?;
    let resp: SandboxListResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp.vms)
}

#[tauri::command]
pub async fn get_sandbox_disk_usage() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let output = tokio::process::Command::new("du")
            .args(["-sh", &format!("{}/.lima", std::env::var("HOME").unwrap_or_default())])
            .output().await.map_err(|e| e.to_string())?;
        let s = String::from_utf8_lossy(&output.stdout);
        Ok(s.split_whitespace().next().unwrap_or("unknown").to_string())
    }
    #[cfg(target_os = "linux")]
    {
        // Use podman system df to get total disk usage
        let output = tokio::process::Command::new("podman")
            .args(["system", "df", "--format", "{{.TotalSize}}"])
            .output().await.map_err(|e| e.to_string())?;
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout);
            // Sum all lines (images, containers, volumes)
            let total = s.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(" + ");
            if total.is_empty() { Ok("0B".to_string()) } else { Ok(total) }
        } else {
            Ok("unknown".to_string())
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Measure the WSL distro storage directory
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        let wsl_dir = format!("{}/.clawenv/wsl", home);
        let path = std::path::Path::new(&wsl_dir);
        if path.exists() {
            // Use PowerShell to get directory size
            let output = silent_cmd("powershell")
                .args(["-Command", &format!(
                    "(Get-ChildItem -Recurse '{}' -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum / 1GB | ForEach-Object {{ '{{0:N1}} GB' -f $_ }}",
                    wsl_dir.replace('/', "\\")
                )])
                .output().await.map_err(|e| e.to_string())?;
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if s.is_empty() { Ok("0 GB".to_string()) } else { Ok(s) }
        } else {
            Ok("0 GB".to_string())
        }
    }
}

/// Perform an action on a sandbox VM (start/stop/delete)
#[tauri::command]
pub async fn sandbox_vm_action(vm_name: String, action: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let args = match action.as_str() {
            "start" => vec!["start", &vm_name],
            "stop" => vec!["stop", &vm_name],
            "delete" => vec!["delete", "--force", &vm_name],
            _ => return Err(format!("Unknown action: {action}")),
        };
        let status = tokio::process::Command::new("limactl")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("limactl {} {} failed", action, vm_name));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let args = match action.as_str() {
            "start" => vec!["start".to_string(), vm_name.clone()],
            "stop" => vec!["stop".to_string(), vm_name.clone()],
            "delete" => vec!["rm".to_string(), "-f".to_string(), vm_name.clone()],
            _ => return Err(format!("Unknown action: {action}")),
        };
        let status = tokio::process::Command::new("podman")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("podman {} {} failed", action, vm_name));
        }
    }
    #[cfg(target_os = "windows")]
    {
        match action.as_str() {
            "start" => {
                silent_cmd("wsl")
                    .args(["--distribution", &vm_name])
                    .stdout(std::process::Stdio::null())
                    .status().await.map_err(|e| e.to_string())?;
            }
            "stop" => {
                silent_cmd("wsl")
                    .args(["--terminate", &vm_name])
                    .status().await.map_err(|e| e.to_string())?;
            }
            "delete" => {
                silent_cmd("wsl")
                    .args(["--unregister", &vm_name])
                    .status().await.map_err(|e| e.to_string())?;
            }
            _ => return Err(format!("Unknown action: {action}")),
        }
    }
    tracing::info!("Sandbox VM action: {} on {}", action, vm_name);
    Ok(())
}

/// Check if Chromium is installed in a sandbox instance
#[tauri::command]
pub async fn check_chromium_installed(name: String) -> Result<bool, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let result = backend.exec("which chromium 2>/dev/null || which chromium-browser 2>/dev/null || echo ''").await;
    match result {
        Ok(out) => Ok(!out.trim().is_empty()),
        Err(_) => Ok(false),
    }
}

/// Install Chromium + noVNC in a running sandbox instance
#[tauri::command]
pub async fn install_chromium(
    app: tauri::AppHandle,
    name: String,
) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    // Check if already installed
    let already = backend.exec("which chromium 2>/dev/null || which chromium-browser 2>/dev/null || echo ''").await.unwrap_or_default();
    if !already.trim().is_empty() {
        let _ = app.emit("chromium-install-progress", "Chromium is already installed!");
        return Ok(());
    }

    let _ = app.emit("chromium-install-progress", "Installing Chromium and dependencies (~630MB)...");
    let _ = app.emit("chromium-install-progress", "Note: apk will resume from any previously downloaded packages.");

    // Use exec_with_progress for streaming output
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
    let app2 = app.clone();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            let _ = app2.emit("chromium-install-progress", &line);
        }
    });

    let result = backend.exec_with_progress(
        "sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont 2>&1 || apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont 2>&1",
        &tx,
    ).await;
    drop(tx);

    match result {
        Ok(output) => {
            let _ = app.emit("chromium-install-progress", "✓ Chromium installed successfully!");
            tracing::info!("Chromium installed in '{}': {}", name, output.chars().take(200).collect::<String>());
            Ok(())
        }
        Err(e) => {
            let _ = app.emit("chromium-install-progress", &format!("✗ Installation failed: {e}"));
            Err(e.to_string())
        }
    }
}
