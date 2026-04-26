use clawops_core::proxy::ProxyConfig;
use serde::Serialize;
use tauri::Emitter;

/// Test connectivity to multiple endpoints using Tauri event streaming
#[derive(Serialize, Clone)]
pub struct ConnTestResult {
    pub endpoint: String,
    pub ok: bool,
    pub message: String,
}

/// Core connectivity probe. Separated from the Tauri command so install
/// pre-flight can reuse the exact same endpoint list and client-build logic
/// without any special-casing. `app` is optional — when Some, each endpoint's
/// progress is streamed via `conn-test-step`; when None (pre-flight use) the
/// probe runs silently and only the return value matters.
///
/// Rationale: v0.3.0 makes connectivity a gate, not a "helpful indicator".
/// Both the explicit wizard test and the pre-install gate must agree on what
/// "reachable" means — one shared implementation avoids drift between them.
pub async fn run_connectivity_probes(
    app: Option<&tauri::AppHandle>,
    proxy_json: Option<&str>,
) -> Result<Vec<ConnTestResult>, String> {
    let client = if let Some(pj) = proxy_json {
        if let Ok(proxy) = serde_json::from_str::<ProxyConfig>(pj) {
            if proxy.enabled && !proxy.http_proxy.is_empty() {
                let rp = reqwest::Proxy::all(&proxy.http_proxy).map_err(|e| e.to_string())?;
                reqwest::Client::builder()
                    .proxy(rp)
                    .no_proxy()
                    .build()
                    .map_err(|e| e.to_string())?
            } else {
                reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .map_err(|e| e.to_string())?
            }
        } else {
            reqwest::Client::new()
        }
    } else {
        reqwest::Client::new()
    };

    // Endpoints chosen so each covers a distinct failure mode: npm
    // (Cloudflare — usually works even on restricted networks), github
    // (classic restricted target), nodejs.org (dist downloads), alpine
    // CDN (sandbox apk). npm + github alone gives false "ready" in
    // networks where only github is blocked — keep all four.
    let endpoints = vec![
        ("npm Registry", "https://registry.npmjs.org/"),
        ("GitHub", "https://api.github.com/"),
        ("Node.js dist", "https://nodejs.org/dist/"),
        ("Alpine CDN", "https://dl-cdn.alpinelinux.org/alpine/latest-stable/"),
    ];

    let mut results = Vec::new();
    for (name, url) in &endpoints {
        if let Some(a) = app {
            let _ = a.emit("conn-test-step", serde_json::json!({
                "endpoint": name, "status": "testing"
            }));
        }

        let res = client
            .get(*url)
            .header("User-Agent", "ClawEnv/0.1")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await;
        let (ok, msg) = match res {
            Ok(r) if r.status().is_success() || r.status().is_redirection() => {
                (true, format!("OK ({})", r.status()))
            }
            Ok(r) => (false, format!("HTTP {}", r.status())),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("timed out") {
                    (false, "Timeout (15s)".to_string())
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
        if let Some(a) = app {
            let _ = a.emit("conn-test-step", serde_json::json!({
                "endpoint": name,
                "status": if result.ok { "ok" } else { "fail" },
                "message": &result.message,
            }));
        }
        results.push(result);
    }
    Ok(results)
}

#[tauri::command]
pub async fn test_connectivity(
    app: tauri::AppHandle,
    proxy_json: Option<String>,
) -> Result<Vec<ConnTestResult>, String> {
    run_connectivity_probes(Some(&app), proxy_json.as_deref()).await
}

/// Synchronous, blocking variant of `detect_system_proxy` suitable for use
/// from `main()` before any async runtime is up. Returns the detected proxy
/// payload (same JSON shape the Tauri command returns) so callers can pick
/// out just the fields they need.
///
/// Skips env vars — the caller's intent at startup is "find the OS-level
/// system proxy and inject it into our env". If env is already set, nothing
/// to inject; this helper ignores env and only queries the native store.
pub fn detect_system_proxy_native_only() -> Option<serde_json::Value> {
    #[cfg(target_os = "macos")]
    { return detect_macos_proxy(); }
    #[cfg(target_os = "windows")]
    { return detect_windows_proxy(); }
    #[allow(unreachable_code)]
    None
}

/// Test connectivity TO a set of targets FROM inside a specific sandbox VM.
/// Used by the ProxyModal "test" button to verify the proxy applied to
/// the VM actually reaches the targets the user cares about (LLM APIs,
/// npm, github, etc.). See docs/23-proxy-architecture.md §11.
///
/// Preset keys (`targets` strings): `github` / `npm` / `openai` /
/// `anthropic` / `deepseek` / `qwen`. Any other value is treated as a
/// literal URL (`https://...` expected).
#[tauri::command]
pub async fn test_instance_network(
    name: String,
    targets: Vec<String>,
) -> Result<Vec<serde_json::Value>, String> {
    use clawops_core::instance::InstanceRegistry;

    let registry = InstanceRegistry::with_default_path();
    let inst = registry.find(&name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Instance '{name}' not found"))?;

    let backend = crate::instance_helper::backend_for_instance(&inst)?;

    let mut results = Vec::new();
    for t in targets {
        let url = preset_url(&t).unwrap_or(&t).to_string();
        let cmd = format!(
            "curl -sS -m 8 -o /dev/null -w '%{{http_code}}|%{{time_total}}' '{}' 2>&1 || echo 'FAIL|0'",
            url.replace('\'', "'\\''"),
        );
        let out = backend.exec(&cmd).await.unwrap_or_else(|e| format!("FAIL|0|{e}"));
        let parts: Vec<&str> = out.trim().split('|').collect();
        let (ok, code, latency) = if parts.len() >= 2 {
            let code = parts[0].to_string();
            let ok = code.starts_with('2') || code.starts_with('3');
            let latency = parts[1].parse::<f64>().unwrap_or(0.0);
            (ok, code, latency)
        } else {
            (false, "FAIL".to_string(), 0.0)
        };
        results.push(serde_json::json!({
            "target": t,
            "url": url,
            "ok": ok,
            "http_code": code,
            "latency_ms": (latency * 1000.0).round(),
        }));
    }
    Ok(results)
}

fn preset_url(key: &str) -> Option<&'static str> {
    match key {
        "github"    => Some("https://api.github.com"),
        "npm"       => Some("https://registry.npmjs.org"),
        "openai"    => Some("https://api.openai.com"),
        "anthropic" => Some("https://api.anthropic.com"),
        "deepseek"  => Some("https://api.deepseek.com"),
        "qwen"      => Some("https://dashscope.aliyuncs.com"),
        "npmmirror" => Some("https://registry.npmmirror.com"),
        _ => None,
    }
}

/// Detect system proxy across platforms.
///
/// Order of precedence:
///   1. `HTTP_PROXY` / `HTTPS_PROXY` env vars (what the GUI itself sees).
///   2. Platform-native source:
///        - macOS  — `SCDynamicStoreCopyProxies` via system-configuration
///                    crate; covers every active interface (Wi-Fi / Ethernet
///                    / Thunderbolt / USB LAN), not just hardcoded Wi-Fi.
///        - Windows — `HKCU\...\Internet Settings` registry (Internet Options).
///        - Linux   — `gsettings get org.gnome.system.proxy` (GNOME); other
///                    DEs typically rely on env vars, already covered in (1).
///
/// PAC auto-config URLs are reported via `source` = "macos-pac" so the wizard
/// can show a "PAC detected — please fill manually" hint (we don't resolve
/// PAC scripts). SOCKS-only proxies are reported too but flagged in `source`
/// since reqwest needs its `socks` feature to dial them, which we don't
/// enable by default.
#[tauri::command]
pub async fn detect_system_proxy() -> Result<serde_json::Value, String> {
    // 1. Env vars — what the GUI process itself sees.
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

    // 2a. macOS — SystemConfiguration framework.
    #[cfg(target_os = "macos")]
    if let Some(v) = detect_macos_proxy() {
        return Ok(v);
    }

    // 2b. Windows — registry.
    #[cfg(target_os = "windows")]
    if let Some(v) = detect_windows_proxy() {
        return Ok(v);
    }

    Ok(serde_json::json!({
        "detected": false,
        "source": "none",
        "http_proxy": "",
        "https_proxy": "",
        "no_proxy": "",
    }))
}

/// macOS proxy detection via SystemConfiguration framework — returns the
/// current active interface's proxy settings, not a hardcoded Wi-Fi lookup.
///
/// Returns `None` when no proxy is configured, so the caller can fall through
/// to the "none detected" response. Returns `Some(...)` even for PAC / SOCKS
/// so the UI can show a friendly hint even when the value isn't directly
/// usable by reqwest.
#[cfg(target_os = "macos")]
fn detect_macos_proxy() -> Option<serde_json::Value> {
    use core_foundation::array::{CFArray, CFArrayGetCount, CFArrayGetValueAtIndex};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use std::os::raw::c_void;
    use system_configuration::dynamic_store::SCDynamicStoreBuilder;

    // build() can fail if CFRunLoop isn't set up (very rare — returns None).
    let store = SCDynamicStoreBuilder::new("clawenv-proxy-detect").build()?;
    let proxies: CFDictionary<CFString, CFType> = store.get_proxies()?;

    // Helpers. `proxies.find` returns `ItemRef<CFType>` which derefs to
    // `&CFType`. We bind through `&CFType` explicitly so the `downcast`
    // turbofish resolves unambiguously to `CFType::downcast<T>` instead of
    // getting tangled in method resolution with `CFPropertyList::downcast`.
    let get_string = |k: &str| -> Option<String> {
        let key = CFString::new(k);
        let v = proxies.find(&key)?;
        let cft: &CFType = &v;
        let s: CFString = cft.downcast::<CFString>()?;
        Some(s.to_string())
    };
    let get_num = |k: &str| -> Option<i64> {
        let key = CFString::new(k);
        let v = proxies.find(&key)?;
        let cft: &CFType = &v;
        let n: CFNumber = cft.downcast::<CFNumber>()?;
        n.to_i64()
    };
    let get_bool = |k: &str| -> bool {
        get_num(k).map(|n| n != 0).unwrap_or(false)
    };
    let get_u16 = |k: &str| -> Option<u16> {
        get_num(k).and_then(|n| u16::try_from(n).ok())
    };
    // CFArray downcast is only impl'd for `CFArray<*const c_void>` (the
    // heterogeneous form) — ConcreteCFType isn't impl'd for the typed
    // `CFArray<CFString>`. We downcast to the untyped form and pull each
    // element out via the raw FFI, wrapping as CFString (ExceptionsList is
    // documented to always contain strings).
    let get_strings = |k: &str| -> Vec<String> {
        let key = CFString::new(k);
        let Some(v) = proxies.find(&key) else { return Vec::new() };
        let cft: &CFType = &v;
        let Some(arr) = cft.downcast::<CFArray<*const c_void>>() else { return Vec::new() };
        let raw = arr.as_concrete_TypeRef();
        let count = unsafe { CFArrayGetCount(raw) };
        (0..count)
            .map(|i| unsafe {
                let item_ptr = CFArrayGetValueAtIndex(raw, i) as CFStringRef;
                CFString::wrap_under_get_rule(item_ptr).to_string()
            })
            .collect()
    };

    // Exceptions list → NO_PROXY string. `localhost,127.0.0.1` always merged
    // in so the wizard's connectivity tests to `127.0.0.1` don't detour.
    let mut no_proxy_parts = get_strings("ExceptionsList");
    for d in ["localhost", "127.0.0.1"] {
        if !no_proxy_parts.iter().any(|s| s == d) {
            no_proxy_parts.push(d.to_string());
        }
    }
    let no_proxy = no_proxy_parts.join(",");

    // HTTPS preferred over HTTP when both are configured — HTTPS requests
    // (most of what we do) should use the HTTPS proxy.
    let https_on = get_bool("HTTPSEnable");
    let http_on  = get_bool("HTTPEnable");

    let https_url = if https_on {
        let host = get_string("HTTPSProxy").unwrap_or_default();
        let port = get_u16("HTTPSPort").unwrap_or(0);
        (!host.is_empty() && port != 0).then(|| format!("http://{host}:{port}"))
    } else { None };

    let http_url = if http_on {
        let host = get_string("HTTPProxy").unwrap_or_default();
        let port = get_u16("HTTPPort").unwrap_or(0);
        (!host.is_empty() && port != 0).then(|| format!("http://{host}:{port}"))
    } else { None };

    if https_url.is_some() || http_url.is_some() {
        // Fall back each direction if one is missing.
        let h = http_url.clone().or_else(|| https_url.clone()).unwrap();
        let s = https_url.or(http_url).unwrap();
        return Some(serde_json::json!({
            "detected": true,
            "source": "macOS System Preferences",
            "http_proxy": h,
            "https_proxy": s,
            "no_proxy": no_proxy,
        }));
    }

    // PAC auto-config — we don't resolve it, but tell the user we saw one.
    if get_bool("ProxyAutoConfigEnable") {
        if let Some(pac) = get_string("ProxyAutoConfigURLString") {
            return Some(serde_json::json!({
                "detected": false,
                "source": "macos-pac",
                "http_proxy": "",
                "https_proxy": "",
                "no_proxy": no_proxy,
                "pac_url": pac,
                "note": "PAC (auto-config) URL detected. PAC scripts are not resolved — please switch to an explicit HTTP proxy or enter one manually.",
            }));
        }
    }

    // SOCKS-only: we can detect but reqwest without socks feature can't dial
    // it. Report so the UI can prompt the user.
    if get_bool("SOCKSEnable") {
        let host = get_string("SOCKSProxy").unwrap_or_default();
        let port = get_u16("SOCKSPort").unwrap_or(0);
        if !host.is_empty() && port != 0 {
            return Some(serde_json::json!({
                "detected": false,
                "source": "macos-socks",
                "http_proxy": "",
                "https_proxy": "",
                "no_proxy": no_proxy,
                "socks_proxy": format!("socks5://{host}:{port}"),
                "note": "SOCKS proxy detected — ClawEnv cannot dial it directly. Please configure an HTTP/HTTPS proxy or use a local HTTP bridge (Clash/V2Ray usually provides one).",
            }));
        }
    }

    None
}

/// Windows proxy detection via `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`.
/// Reads `ProxyEnable`, `ProxyServer` (supports both single "host:port" and
/// "http=host:port;https=host:port" forms), and `ProxyOverride` (bypass list).
#[cfg(target_os = "windows")]
fn detect_windows_proxy() -> Option<serde_json::Value> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enable: u32 = key.get_value("ProxyEnable").ok()?;
    if enable == 0 {
        return None;
    }

    let server: String = key.get_value("ProxyServer").ok()?;
    let override_raw: String = key.get_value("ProxyOverride").unwrap_or_default();

    // ProxyServer can be either `host:port` (same for all) or
    // `http=host:port;https=host:port;ftp=host:port`.
    let (http_url, https_url) = if server.contains('=') {
        let mut http = String::new();
        let mut https = String::new();
        for part in server.split(';') {
            if let Some(v) = part.strip_prefix("http=") { http = format!("http://{v}"); }
            if let Some(v) = part.strip_prefix("https=") { https = format!("http://{v}"); }
        }
        (http, https)
    } else {
        let u = format!("http://{server}");
        (u.clone(), u)
    };
    let http  = if http_url.is_empty()  { https_url.clone() } else { http_url };
    let https = if https_url.is_empty() { http.clone() } else { https_url };

    // `ProxyOverride` uses semicolons and often includes `<local>`. Translate
    // into a comma-separated list for NO_PROXY; drop the Windows-specific
    // `<local>` since reqwest doesn't understand it — rely on explicit
    // `localhost,127.0.0.1`.
    let mut no_proxy: Vec<String> = override_raw
        .split(';')
        .filter(|s| !s.is_empty() && *s != "<local>")
        .map(|s| s.trim().to_string())
        .collect();
    for d in ["localhost", "127.0.0.1"] {
        if !no_proxy.iter().any(|s| s == d) {
            no_proxy.push(d.to_string());
        }
    }

    Some(serde_json::json!({
        "detected": true,
        "source": "Windows Registry (Internet Settings)",
        "http_proxy": http,
        "https_proxy": https,
        "no_proxy": no_proxy.join(","),
    }))
}

