use clawenv_core::config::{ConfigManager, ProxyConfig, UserMode};
use clawenv_core::launcher::{self, LaunchState};
use clawenv_core::manager::{install, instance};
use clawenv_core::sandbox::InstallMode;
use serde::Serialize;
use tauri::Emitter;

#[tauri::command]
pub async fn detect_launch_state() -> Result<LaunchState, String> {
    launcher::detect_launch_state()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_openclaw_url(instance_name: Option<String>) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let name = instance_name.unwrap_or_else(|| "default".into());

    let inst = config
        .instances()
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| format!("Instance '{}' not found", name))?;

    Ok(format!("http://127.0.0.1:{}", inst.openclaw.gateway_port))
}

#[derive(Debug, Serialize)]
pub struct InstanceInfo {
    pub name: String,
    pub sandbox_type: String,
    pub version: String,
    pub gateway_port: u16,
}

#[tauri::command]
pub fn list_instances() -> Result<Vec<InstanceInfo>, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;

    Ok(config
        .instances()
        .iter()
        .map(|inst| InstanceInfo {
            name: inst.name.clone(),
            sandbox_type: format!("{:?}", inst.sandbox_type),
            version: inst.version.clone(),
            gateway_port: inst.openclaw.gateway_port,
        })
        .collect())
}

#[derive(Serialize)]
pub struct InstanceStatusDetail {
    pub processes: String,
    pub resources: String,
    pub gateway_log: String,
}

#[tauri::command]
pub async fn get_instance_status_detail(name: String) -> Result<InstanceStatusDetail, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    let processes = backend.exec(
        "ps aux 2>/dev/null || ps -ef 2>/dev/null || echo 'ps not available'"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));

    let resources = backend.exec(
        "echo '--- Memory ---' && free -m 2>/dev/null || cat /proc/meminfo 2>/dev/null | head -5; echo ''; echo '--- Disk ---' && df -h / 2>/dev/null; echo ''; echo '--- Uptime ---' && uptime 2>/dev/null"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));

    // Read the actual running gateway log (not the startup wrapper log)
    let gateway_log = backend.exec(
        "cat /tmp/openclaw/openclaw-*.log 2>/dev/null | tail -100 || tail -80 /tmp/openclaw-gateway.log 2>/dev/null || echo 'No gateway log found'"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));

    Ok(InstanceStatusDetail { processes, resources, gateway_log })
}

#[tauri::command]
pub async fn get_instance_logs(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let log = backend.exec(
        "cat /tmp/openclaw/openclaw-*.log 2>/dev/null | tail -100 || tail -100 /tmp/openclaw-gateway.log 2>/dev/null || echo 'No gateway log'"
    ).await.unwrap_or_else(|e| format!("Error: {e}"));
    Ok(log)
}

#[tauri::command]
pub async fn install_openclaw(
    app: tauri::AppHandle,
    instance_name: String,
    claw_version: String,
    api_key: Option<String>,
    use_native: bool,
    install_browser: bool,
    gateway_port: u16,
) -> Result<(), String> {
    let opts = install::InstallOptions {
        instance_name,
        claw_version,
        install_mode: InstallMode::OnlineBuild,
        install_browser,
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

#[tauri::command]
pub async fn start_instance(name: String) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    instance::start_instance(inst).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_instance(name: String) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    instance::stop_instance(inst).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_instance_health(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let health = instance::instance_health(inst).await;
    // Return snake_case to match serde serialization in monitor events
    let result = match health {
        clawenv_core::monitor::InstanceHealth::Running => "running",
        clawenv_core::monitor::InstanceHealth::Stopped => "stopped",
        clawenv_core::monitor::InstanceHealth::Unreachable => "unreachable",
    };
    tracing::info!("get_instance_health('{}') = {}", name, result);
    Ok(result.to_string())
}

#[tauri::command]
pub async fn save_settings(settings_json: String) -> Result<(), String> {
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;

    // Parse the incoming JSON as partial config fields
    let values: serde_json::Value =
        serde_json::from_str(&settings_json).map_err(|e| e.to_string())?;

    let cfg = config.config_mut();

    if let Some(lang) = values.get("language").and_then(|v| v.as_str()) {
        cfg.clawenv.language = lang.to_string();
    }
    if let Some(theme) = values.get("theme").and_then(|v| v.as_str()) {
        cfg.clawenv.theme = theme.to_string();
    }
    if let Some(proxy) = values.get("proxy") {
        if let Ok(p) = serde_json::from_value::<ProxyConfig>(proxy.clone()) {
            // Store proxy password in keychain if present in JSON
            if let Some(password) = proxy.get("auth_password").and_then(|v| v.as_str()) {
                if !password.is_empty() {
                    let _ = clawenv_core::config::keychain::store_proxy_password(password);
                }
            }
            cfg.clawenv.proxy = p;
        }
    }
    if let Some(auto_check) = values.get("auto_check_updates").and_then(|v| v.as_bool()) {
        cfg.clawenv.updates.auto_check = auto_check;
    }

    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_proxy(proxy_json: String) -> Result<(), String> {
    let proxy: ProxyConfig =
        serde_json::from_str(&proxy_json).map_err(|e| e.to_string())?;
    clawenv_core::config::proxy::test_proxy(&proxy)
        .await
        .map_err(|e| e.to_string())
}

/// Test connectivity to multiple endpoints using Tauri event streaming
#[derive(Serialize, Clone)]
pub struct ConnTestResult {
    pub endpoint: String,
    pub ok: bool,
    pub message: String,
}

#[tauri::command]
pub async fn test_connectivity(
    app: tauri::AppHandle,
    proxy_json: Option<String>,
) -> Result<Vec<ConnTestResult>, String> {
    // Build client: if proxy specified use it, otherwise use system default
    let client = if let Some(ref pj) = proxy_json {
        if let Ok(proxy) = serde_json::from_str::<ProxyConfig>(pj) {
            if proxy.enabled && !proxy.http_proxy.is_empty() {
                // Explicit proxy — disable system proxy auto-detection
                let rp = reqwest::Proxy::all(&proxy.http_proxy).map_err(|e| e.to_string())?;
                reqwest::Client::builder()
                    .proxy(rp)
                    .no_proxy()  // don't also use system proxy
                    .build()
                    .map_err(|e| e.to_string())?
            } else {
                // "none" mode — no proxy at all
                reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .map_err(|e| e.to_string())?
            }
        } else {
            // Use system default (reqwest auto-detects HTTP_PROXY etc)
            reqwest::Client::new()
        }
    } else {
        // null = use system defaults
        reqwest::Client::new()
    };

    let endpoints = vec![
        ("Alpine CDN", "https://dl-cdn.alpinelinux.org/alpine/latest-stable/"),
        ("npm Registry", "https://registry.npmjs.org/"),
        ("GitHub API", "https://api.github.com/"),
        ("OpenClaw Registry", "https://registry.npmjs.org/openclaw"),
    ];

    let mut results = Vec::new();
    for (name, url) in &endpoints {
        // Emit each step as it starts
        let _ = app.emit("conn-test-step", serde_json::json!({
            "endpoint": name, "status": "testing"
        }));

        let res = client
            .get(*url)
            .header("User-Agent", "ClawEnv/0.1")
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await;
        let (ok, msg) = match res {
            Ok(r) if r.status().is_success() || r.status().is_redirection() => {
                (true, format!("OK ({})", r.status()))
            }
            Ok(r) => (false, format!("HTTP {}", r.status())),
            Err(e) => {
                let err_str = e.to_string();
                // Give friendlier messages
                if err_str.contains("timed out") {
                    (false, "Timeout (8s)".to_string())
                } else if err_str.contains("dns") || err_str.contains("resolve") {
                    (false, "DNS resolution failed".to_string())
                } else if err_str.contains("connect") {
                    (false, "Connection refused".to_string())
                } else {
                    (false, err_str)
                }
            }
        };

        let result = ConnTestResult {
            endpoint: name.to_string(),
            ok,
            message: msg,
        };
        // Emit each result as it completes
        let _ = app.emit("conn-test-step", serde_json::json!({
            "endpoint": name,
            "status": if result.ok { "ok" } else { "fail" },
            "message": &result.message,
        }));
        results.push(result);
    }
    Ok(results)
}

/// Detect system proxy — check env vars + macOS networksetup
#[tauri::command]
pub async fn detect_system_proxy() -> Result<serde_json::Value, String> {
    // 1. Check environment variables
    let env_http = std::env::var("http_proxy")
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .unwrap_or_default();
    let env_https = std::env::var("https_proxy")
        .or_else(|_| std::env::var("HTTPS_PROXY"))
        .unwrap_or_default();
    let env_no_proxy = std::env::var("no_proxy")
        .or_else(|_| std::env::var("NO_PROXY"))
        .unwrap_or_default();

    if !env_http.is_empty() || !env_https.is_empty() {
        return Ok(serde_json::json!({
            "detected": true,
            "source": "environment",
            "http_proxy": env_http,
            "https_proxy": env_https,
            "no_proxy": env_no_proxy,
        }));
    }

    // 2. macOS: check networksetup for HTTP proxy
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = tokio::process::Command::new("networksetup")
            .args(["-getwebproxy", "Wi-Fi"])
            .output()
            .await
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut enabled = false;
            let mut server = String::new();
            let mut port = String::new();
            for line in stdout.lines() {
                if line.starts_with("Enabled:") && line.contains("Yes") {
                    enabled = true;
                }
                if let Some(s) = line.strip_prefix("Server: ") {
                    server = s.trim().to_string();
                }
                if let Some(p) = line.strip_prefix("Port: ") {
                    port = p.trim().to_string();
                }
            }
            if enabled && !server.is_empty() {
                let proxy_url = format!("http://{}:{}", server, port);
                return Ok(serde_json::json!({
                    "detected": true,
                    "source": "macOS System Preferences (Wi-Fi)",
                    "http_proxy": proxy_url,
                    "https_proxy": proxy_url,
                    "no_proxy": "",
                }));
            }
        }

        // Also check HTTPS proxy
        if let Ok(output) = tokio::process::Command::new("networksetup")
            .args(["-getsecurewebproxy", "Wi-Fi"])
            .output()
            .await
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut enabled = false;
            let mut server = String::new();
            let mut port = String::new();
            for line in stdout.lines() {
                if line.starts_with("Enabled:") && line.contains("Yes") {
                    enabled = true;
                }
                if let Some(s) = line.strip_prefix("Server: ") {
                    server = s.trim().to_string();
                }
                if let Some(p) = line.strip_prefix("Port: ") {
                    port = p.trim().to_string();
                }
            }
            if enabled && !server.is_empty() {
                let proxy_url = format!("http://{}:{}", server, port);
                return Ok(serde_json::json!({
                    "detected": true,
                    "source": "macOS System Preferences (Wi-Fi HTTPS)",
                    "http_proxy": proxy_url,
                    "https_proxy": proxy_url,
                    "no_proxy": "",
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "detected": false,
        "source": "none",
        "http_proxy": "",
        "https_proxy": "",
        "no_proxy": "",
    }))
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

    // Memory (macOS: sysctl hw.memsize)
    let memory_gb = {
        #[cfg(target_os = "macos")]
        {
            let out = tokio::process::Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output().await;
            match out {
                Ok(o) => {
                    let s = String::from_utf8_lossy(&o.stdout);
                    s.trim().parse::<f64>().unwrap_or(0.0) / 1_073_741_824.0
                }
                Err(_) => 0.0,
            }
        }
        #[cfg(not(target_os = "macos"))]
        { 0.0 }
    };

    // Disk free space
    let disk_free_gb = {
        #[cfg(target_os = "macos")]
        {
            let out = tokio::process::Command::new("df")
                .args(["-g", "/"])
                .output().await;
            match out {
                Ok(o) => {
                    let s = String::from_utf8_lossy(&o.stdout);
                    // Parse "Available" column from df output
                    s.lines().nth(1)
                        .and_then(|line| line.split_whitespace().nth(3))
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                }
                Err(_) => 0.0,
            }
        }
        #[cfg(not(target_os = "macos"))]
        { 0.0 }
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

/// Read the gateway auth token from inside the sandbox
#[tauri::command]
pub async fn get_gateway_token(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let result = backend.exec(
        "cat ~/.openclaw/openclaw.json 2>/dev/null | grep -o '\"token\":[ ]*\"[^\"]*\"' | head -1 | sed 's/.*\"\\([^\"]*\\)\"/\\1/'"
    ).await.map_err(|e| e.to_string())?;
    Ok(result.trim().to_string())
}

/// Get bridge server configuration
#[tauri::command]
pub fn get_bridge_config() -> Result<serde_json::Value, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let bridge = &config.config().clawenv.bridge;
    serde_json::to_value(bridge).map_err(|e| e.to_string())
}

/// Update bridge server configuration
#[tauri::command]
pub async fn save_bridge_config(bridge_json: String) -> Result<(), String> {
    let bridge: clawenv_core::config::BridgeConfig =
        serde_json::from_str(&bridge_json).map_err(|e| e.to_string())?;
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;
    config.config_mut().clawenv.bridge = bridge;
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_url_in_browser(url: String) -> Result<(), String> {
    // Fallback: use Rust std to open URL
    #[cfg(target_os = "macos")]
    {
        tokio::process::Command::new("open")
            .arg(&url)
            .status()
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        tokio::process::Command::new("cmd")
            .args(["/c", "start", &url])
            .status()
            .await
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        tokio::process::Command::new("xdg-open")
            .arg(&url)
            .status()
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn create_default_config(user_mode: String) -> Result<(), String> {
    let mode = match user_mode.to_lowercase().as_str() {
        "developer" | "dev" => UserMode::Developer,
        _ => UserMode::General,
    };
    ConfigManager::create_default(mode).map_err(|e| e.to_string())?;
    Ok(())
}
