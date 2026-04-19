use anyhow::Result;

use super::models::{InstanceProxyConfig, ProxyConfig};
use super::keychain;
use crate::sandbox::{SandboxBackend, SandboxType};

/// Build the full proxy URL including auth credentials if needed.
/// `pub(super)` so `proxy_resolver` (sibling module) can use it without
/// reaching into private internals.
pub(super) fn proxy_url_with_auth(proxy: &ProxyConfig) -> Result<String> {
    if !proxy.auth_required || proxy.auth_user.is_empty() {
        return Ok(proxy.http_proxy.clone());
    }

    let password = match keychain::get_proxy_password() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to retrieve proxy password from keychain: {e}. Using empty password.");
            String::new()
        }
    };
    // Insert user:pass into proxy URL: http://user:pass@host:port
    if let Some(rest) = proxy.http_proxy.strip_prefix("http://") {
        Ok(format!("http://{}:{}@{}", proxy.auth_user, password, rest))
    } else if let Some(rest) = proxy.http_proxy.strip_prefix("https://") {
        Ok(format!("https://{}:{}@{}", proxy.auth_user, password, rest))
    } else {
        Ok(format!("http://{}:{}@{}", proxy.auth_user, password, proxy.http_proxy))
    }
}

/// Apply proxy configuration inside a sandbox environment
pub async fn apply_proxy(backend: &dyn SandboxBackend, proxy: &ProxyConfig) -> Result<()> {
    if !proxy.enabled || proxy.http_proxy.is_empty() {
        return Ok(());
    }

    let http_proxy = proxy_url_with_auth(proxy)?;
    let https_proxy = if proxy.https_proxy.is_empty() {
        http_proxy.clone()
    } else {
        proxy.https_proxy.clone()
    };
    let no_proxy = if proxy.no_proxy.is_empty() {
        "localhost,127.0.0.1"
    } else {
        &proxy.no_proxy
    };

    // Write /etc/profile.d/proxy.sh — applies to all processes in sandbox.
    // Escape double quotes in proxy URLs to prevent shell injection.
    let esc_dq = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"export http_proxy="{}"
export https_proxy="{}"
export HTTP_PROXY="{}"
export HTTPS_PROXY="{}"
export no_proxy="{}"
export NO_PROXY="{}"
"#,
        esc_dq(&http_proxy), esc_dq(&https_proxy),
        esc_dq(&http_proxy), esc_dq(&https_proxy),
        esc_dq(no_proxy), esc_dq(no_proxy),
    );

    backend.exec(&format!(
        "cat > /etc/profile.d/proxy.sh << 'PROXYEOF'\n{script}PROXYEOF"
    )).await?;
    backend.exec("chmod +x /etc/profile.d/proxy.sh").await?;

    // Configure npm proxy separately (escape for shell safety)
    let esc = |s: &str| s.replace('\'', "'\\''");
    backend.exec(&format!("npm config set proxy '{}'", esc(&http_proxy))).await?;
    backend.exec(&format!("npm config set https-proxy '{}'", esc(&https_proxy))).await?;

    tracing::info!("Proxy configuration applied to sandbox");
    Ok(())
}

/// **Deprecated**: use `proxy_resolver::Scope::*.resolve()` instead. Kept
/// for the one remaining caller (apply_proxy above, legacy path); all new
/// code must go through the resolver. See docs/23-proxy-architecture.md §6.
#[allow(dead_code)]
pub fn resolve_effective_proxy(proxy: &ProxyConfig) -> Option<(String, String, String)> {
    // Config takes priority — if explicitly enabled, use it.
    if proxy.enabled && !proxy.http_proxy.is_empty() {
        let http = proxy_url_with_auth(proxy).unwrap_or_else(|_| proxy.http_proxy.clone());
        let https = if proxy.https_proxy.is_empty() { http.clone() } else { proxy.https_proxy.clone() };
        let no_proxy = if proxy.no_proxy.is_empty() {
            "localhost,127.0.0.1".to_string()
        } else {
            proxy.no_proxy.clone()
        };
        return Some((http, https, no_proxy));
    }

    // Env fallback — Tauri injects HTTPS_PROXY/HTTP_PROXY into the clawcli
    // subprocess during install when the user picks a proxy in the wizard
    // without persisting it. Upper case wins; fall back to lower case.
    let env_http = std::env::var("HTTP_PROXY").ok()
        .or_else(|| std::env::var("http_proxy").ok())
        .filter(|s| !s.is_empty());
    let env_https = std::env::var("HTTPS_PROXY").ok()
        .or_else(|| std::env::var("https_proxy").ok())
        .filter(|s| !s.is_empty());

    if env_http.is_none() && env_https.is_none() {
        return None;
    }

    let http = env_http.clone().or_else(|| env_https.clone()).unwrap_or_default();
    let https = env_https.unwrap_or_else(|| http.clone());
    let no_proxy = std::env::var("NO_PROXY").ok()
        .or_else(|| std::env::var("no_proxy").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost,127.0.0.1".to_string());

    Some((http, https, no_proxy))
}

/// **Deprecated**: use `proxy_resolver::triple_from_config_proxy` + `apply_env`
/// / `apply_child_cmd`. Kept until all internal call sites migrate.
#[allow(dead_code)]
pub fn build_proxy_env_vars(proxy: &ProxyConfig) -> Vec<(&'static str, String)> {
    if !proxy.enabled || proxy.http_proxy.is_empty() {
        return Vec::new();
    }
    let http_proxy = proxy_url_with_auth(proxy).unwrap_or_else(|_| proxy.http_proxy.clone());
    let https_proxy = if proxy.https_proxy.is_empty() {
        http_proxy.clone()
    } else {
        proxy.https_proxy.clone()
    };
    let no_proxy = if proxy.no_proxy.is_empty() {
        "localhost,127.0.0.1".to_string()
    } else {
        proxy.no_proxy.clone()
    };
    vec![
        ("http_proxy",  http_proxy.clone()),
        ("HTTP_PROXY",  http_proxy),
        ("https_proxy", https_proxy.clone()),
        ("HTTPS_PROXY", https_proxy),
        ("no_proxy",    no_proxy.clone()),
        ("NO_PROXY",    no_proxy),
    ]
}

/// **Deprecated**: use `proxy_resolver::apply_env(&triple)`.
#[allow(dead_code)]
pub fn inject_proxy_env(proxy: &ProxyConfig) {
    let pairs = build_proxy_env_vars(proxy);
    if pairs.is_empty() {
        return;
    }
    for (k, v) in &pairs {
        std::env::set_var(k, v);
    }
    if let Some((_, http)) = pairs.iter().find(|(k, _)| *k == "HTTP_PROXY") {
        tracing::debug!("Proxy env injected: http={http}");
    }
}

/// **Deprecated**: use `proxy_resolver::sandbox_host_address` instead.
#[allow(dead_code)]
pub async fn sandbox_host_address(
    backend: &dyn SandboxBackend,
    sandbox_type: SandboxType,
) -> Result<String> {
    match sandbox_type {
        SandboxType::LimaAlpine => Ok("host.lima.internal".into()),
        SandboxType::PodmanAlpine => Ok("host.containers.internal".into()),
        SandboxType::Wsl2Alpine => {
            // `resolv.conf` is regenerated on each WSL boot unless the user
            // disables `generateResolvConf` — in the common case the first
            // nameserver is the Windows host's virtual adapter IP.
            let out = backend
                .exec("grep -oE '^nameserver[[:space:]]+[0-9.]+' /etc/resolv.conf | head -1 | awk '{print $2}'")
                .await
                .unwrap_or_default();
            let ip = out.trim().to_string();
            if ip.is_empty() {
                // Last-ditch fallback: some Windows setups expose
                // `host.docker.internal` via Docker Desktop's WSL hook.
                Ok("host.docker.internal".into())
            } else {
                Ok(ip)
            }
        }
        SandboxType::Native => Ok("127.0.0.1".into()),
    }
}

/// Rewrite a host-loopback proxy URL to a VM-reachable form. Non-loopback
/// URLs (e.g. `http://192.168.1.10:7890`, `http://proxy.corp:3128`) pass
/// through untouched — they already reach the host from the VM's network.
///
/// Called by the "sync host proxy" code path: the user's host proxy is
/// typically `http://127.0.0.1:7890` (Clash / Surge / etc.), which the
/// sandbox cannot dial directly. We substitute 127.0.0.1 / localhost with
/// the backend's host address.
#[allow(dead_code)] // superseded by proxy_resolver::rewrite_loopback
pub fn rewrite_proxy_url_for_sandbox(url: &str, host_addr: &str) -> String {
    if host_addr == "127.0.0.1" {
        return url.to_string(); // Native — no translation
    }
    // Substring replace is fine — valid URLs won't contain "127.0.0.1" or
    // "localhost" anywhere except as host part. Userinfo or path segments
    // containing these tokens are exotic enough to not worry about.
    url.replace("127.0.0.1", host_addr)
       .replace("://localhost", &format!("://{host_addr}"))
}

/// Apply per-instance proxy config to a running sandbox. Rewrites
/// `/etc/profile.d/proxy.sh`, removing it entirely when mode is "none"
/// so the sandbox falls back to direct connection.
///
/// Returns the effective URLs (post-rewrite) so the caller can persist
/// them into `InstanceProxyConfig` for later inspection / export.
#[allow(dead_code)] // superseded by proxy_resolver::apply_to_sandbox
pub async fn apply_instance_proxy(
    backend: &dyn SandboxBackend,
    sandbox_type: SandboxType,
    cfg: &InstanceProxyConfig,
) -> Result<(String, String, String)> {
    if cfg.mode == "none" || cfg.http_proxy.is_empty() {
        // Strip any existing proxy file — running processes won't pick this
        // up until restart, but new shells / new claw invocations will.
        backend.exec("rm -f /etc/profile.d/proxy.sh").await.ok();
        backend.exec("npm config delete proxy 2>/dev/null || true").await.ok();
        backend.exec("npm config delete https-proxy 2>/dev/null || true").await.ok();
        return Ok((String::new(), String::new(), String::new()));
    }

    let host_addr = sandbox_host_address(backend, sandbox_type).await?;
    let http = if cfg.mode == "sync-host" {
        rewrite_proxy_url_for_sandbox(&cfg.http_proxy, &host_addr)
    } else {
        cfg.http_proxy.clone()
    };
    let https = if cfg.https_proxy.is_empty() {
        http.clone()
    } else if cfg.mode == "sync-host" {
        rewrite_proxy_url_for_sandbox(&cfg.https_proxy, &host_addr)
    } else {
        cfg.https_proxy.clone()
    };
    let no_proxy = if cfg.no_proxy.is_empty() {
        "localhost,127.0.0.1".to_string()
    } else {
        cfg.no_proxy.clone()
    };

    let esc_dq = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"export http_proxy="{}"
export https_proxy="{}"
export HTTP_PROXY="{}"
export HTTPS_PROXY="{}"
export no_proxy="{}"
export NO_PROXY="{}"
"#,
        esc_dq(&http), esc_dq(&https),
        esc_dq(&http), esc_dq(&https),
        esc_dq(&no_proxy), esc_dq(&no_proxy),
    );

    backend
        .exec(&format!(
            "cat > /etc/profile.d/proxy.sh << 'PROXYEOF'\n{script}PROXYEOF"
        ))
        .await?;
    backend.exec("chmod +x /etc/profile.d/proxy.sh").await?;

    // npm: separate settings, not inherited from env on `npm install`
    let esc_sq = |s: &str| s.replace('\'', "'\\''");
    backend.exec(&format!("npm config set proxy '{}'", esc_sq(&http))).await.ok();
    backend.exec(&format!("npm config set https-proxy '{}'", esc_sq(&https))).await.ok();

    Ok((http, https, no_proxy))
}

/// Test proxy connectivity by attempting to reach Alpine CDN.
/// `alpine_url` allows testing against a mirror instead of the default CDN.
pub async fn test_proxy(proxy: &ProxyConfig, alpine_url: &str) -> Result<()> {
    let test_url = if alpine_url.is_empty() {
        "https://dl-cdn.alpinelinux.org/alpine/latest-stable/"
    } else {
        alpine_url
    };

    let client = if proxy.enabled && !proxy.http_proxy.is_empty() {
        let proxy_url = proxy_url_with_auth(proxy)?;
        let reqwest_proxy = reqwest::Proxy::all(&proxy_url)?;
        reqwest::Client::builder().proxy(reqwest_proxy).build()?
    } else {
        reqwest::Client::new()
    };

    let resp = client
        .head(test_url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?;

    if resp.status().is_success() || resp.status().is_redirection() {
        Ok(())
    } else {
        anyhow::bail!("Proxy test failed: HTTP {}", resp.status())
    }
}
