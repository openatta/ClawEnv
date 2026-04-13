use anyhow::Result;

use super::models::ProxyConfig;
use super::keychain;
use crate::sandbox::SandboxBackend;

/// Build the full proxy URL including auth credentials if needed
fn proxy_url_with_auth(proxy: &ProxyConfig) -> Result<String> {
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

    // Write /etc/profile.d/proxy.sh — applies to all processes in sandbox
    let script = format!(
        r#"export http_proxy="{http_proxy}"
export https_proxy="{https_proxy}"
export HTTP_PROXY="{http_proxy}"
export HTTPS_PROXY="{https_proxy}"
export no_proxy="{no_proxy}"
export NO_PROXY="{no_proxy}"
"#
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

/// Inject proxy configuration into the current process environment.
/// Sets HTTP_PROXY, HTTPS_PROXY, NO_PROXY (and lowercase variants) so that
/// all child processes (npm, curl, etc.) inherit the proxy settings.
/// Called by CLI at startup to ensure proxy is active for all operations.
pub fn inject_proxy_env(proxy: &ProxyConfig) {
    if !proxy.enabled || proxy.http_proxy.is_empty() {
        return;
    }

    let http_proxy = proxy_url_with_auth(proxy).unwrap_or_else(|_| proxy.http_proxy.clone());
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

    std::env::set_var("http_proxy", &http_proxy);
    std::env::set_var("HTTP_PROXY", &http_proxy);
    std::env::set_var("https_proxy", &https_proxy);
    std::env::set_var("HTTPS_PROXY", &https_proxy);
    std::env::set_var("no_proxy", no_proxy);
    std::env::set_var("NO_PROXY", no_proxy);

    tracing::debug!("Proxy env injected: http={}, no_proxy={}", http_proxy, no_proxy);
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
