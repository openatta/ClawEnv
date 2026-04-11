use clawenv_core::config::ProxyConfig;
use serde::Serialize;
use tauri::Emitter;

#[tauri::command]
pub async fn test_proxy(proxy_json: String) -> Result<(), String> {
    let proxy: ProxyConfig =
        serde_json::from_str(&proxy_json).map_err(|e| e.to_string())?;
    clawenv_core::config::proxy::test_proxy(&proxy, "")
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
