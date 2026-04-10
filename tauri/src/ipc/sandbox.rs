use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
use serde::Serialize;
use tauri::Emitter;

/// List all sandbox VMs/containers on the current platform
#[derive(Serialize)]
pub struct SandboxVmInfo {
    pub name: String,
    pub status: String,
    pub cpus: String,
    pub memory: String,
    pub disk: String,
    pub dir_size: String,
    pub managed: bool,
}

#[tauri::command]
pub async fn list_sandbox_vms() -> Result<Vec<SandboxVmInfo>, String> {
    let mut vms = Vec::new();

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let output = tokio::process::Command::new("limactl")
            .args(["list", "--format", "{{.Name}}\t{{.Status}}\t{{.CPUs}}\t{{.Memory}}\t{{.Disk}}\t{{.Dir}}"])
            .output().await.map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 5 {
                // Get actual disk usage of the VM directory
                let dir = parts.get(5).unwrap_or(&"");
                let dir_size = if !dir.is_empty() {
                    let expanded = dir.replace("~", &home);
                    let du = tokio::process::Command::new("du")
                        .args(["-sh", &expanded])
                        .output().await.ok();
                    du.map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap_or("-").to_string())
                        .unwrap_or("-".to_string())
                } else { "-".to_string() };

                vms.push(SandboxVmInfo {
                    name: parts[0].to_string(),
                    status: parts[1].to_string(),
                    cpus: parts[2].to_string(),
                    memory: parts[3].to_string(),
                    disk: parts[4].to_string(),
                    dir_size,
                    managed: parts[0].starts_with("clawenv-"),
                });
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Podman containers — query with --size for actual disk usage
        let output = tokio::process::Command::new("podman")
            .args(["ps", "-a", "--size", "--format", "{{.Names}}\t{{.Status}}\t{{.Size}}"])
            .output().await.map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if !parts.is_empty() {
                let name = parts.first().unwrap_or(&"").to_string();
                let size_str = parts.get(2).unwrap_or(&"-").to_string();

                // Query resource limits via podman inspect
                let (cpus, memory) = if !name.is_empty() {
                    let inspect = tokio::process::Command::new("podman")
                        .args(["inspect", "--format", "{{.HostConfig.NanoCpus}}\t{{.HostConfig.Memory}}", &name])
                        .output().await.ok();
                    if let Some(out) = inspect {
                        let s = String::from_utf8_lossy(&out.stdout);
                        let iparts: Vec<&str> = s.trim().split('\t').collect();
                        let cpu = iparts.first().and_then(|v| v.parse::<u64>().ok())
                            .map(|n| if n > 0 { format!("{}", n / 1_000_000_000) } else { "-".into() })
                            .unwrap_or("-".into());
                        let mem = iparts.get(1).and_then(|v| v.parse::<u64>().ok())
                            .map(|n| if n > 0 { format!("{} MB", n / (1024 * 1024)) } else { "-".into() })
                            .unwrap_or("-".into());
                        (cpu, mem)
                    } else {
                        ("-".into(), "-".into())
                    }
                } else {
                    ("-".into(), "-".into())
                };

                vms.push(SandboxVmInfo {
                    managed: name.starts_with("clawenv-"),
                    name,
                    status: parts.get(1).unwrap_or(&"").to_string(),
                    cpus,
                    memory,
                    disk: size_str.clone(),
                    dir_size: size_str,
                });
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // WSL2 distros
        let output = tokio::process::Command::new("wsl")
            .args(["--list", "--verbose"])
            .output().await.map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        for line in stdout.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let name = parts[0].trim_start_matches('*').trim();

                // Measure distro vhdx disk usage
                let distro_dir = format!("{}\\.clawenv\\wsl\\{}", home.replace('/', "\\"), name);
                let dir_size = if std::path::Path::new(&distro_dir).exists() {
                    let du = tokio::process::Command::new("powershell")
                        .args(["-Command", &format!(
                            "(Get-ChildItem -Recurse '{}' -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum / 1GB | ForEach-Object {{ '{{0:N1}} GB' -f $_ }}",
                            distro_dir
                        )])
                        .output().await.ok();
                    du.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .unwrap_or("-".into())
                } else {
                    "-".to_string()
                };

                vms.push(SandboxVmInfo {
                    name: name.to_string(),
                    status: parts[1].to_string(),
                    cpus: "-".to_string(),
                    memory: "-".to_string(),
                    disk: dir_size.clone(),
                    dir_size,
                    managed: name.starts_with("ClawEnv"),
                });
            }
        }
    }

    Ok(vms)
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
            let output = tokio::process::Command::new("powershell")
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
                tokio::process::Command::new("wsl")
                    .args(["--distribution", &vm_name])
                    .stdout(std::process::Stdio::null())
                    .status().await.map_err(|e| e.to_string())?;
            }
            "stop" => {
                tokio::process::Command::new("wsl")
                    .args(["--terminate", &vm_name])
                    .status().await.map_err(|e| e.to_string())?;
            }
            "delete" => {
                tokio::process::Command::new("wsl")
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
