//! In-VM mirror application: writes `/etc/apk/repositories` and
//! `npm config set registry`. Takes an install-time proxy_on snapshot so
//! callers don't have to re-resolve the proxy chain here.
//!
//! Key design notes:
//!
//!   * apk natively supports multi-line `/etc/apk/repositories` with
//!     ordered fallback (it tries each line in sequence on download
//!     failure). We write every URL from `mirrors_config.alpine_repo_urls`
//!     one after another, so an unreachable first mirror doesn't wedge
//!     the VM — apk retries the next line automatically. This is the
//!     right answer for the E2E-caught SJTU scenario: a proxy that 503s
//!     HTTPS CONNECT to one mirror just makes apk skip to the next.
//!
//!   * npm is single-valued (`npm config set registry <one-url>`). We
//!     preflight-pick the first URL that returns 2xx via a plain HTTP
//!     HEAD, run FROM INSIDE the sandbox (so the check sees the same
//!     network path the actual install will use — including the VM's
//!     proxy env). If no URL is reachable we fall back to the first
//!     candidate and let npm surface the failure.

use anyhow::Result;

use super::models::MirrorsConfig;
use crate::sandbox::SandboxBackend;

/// Apply mirror configuration inside a sandbox environment.
/// Writes `/etc/apk/repositories` and optionally `npm config set registry`.
pub async fn apply_mirrors(
    backend: &dyn SandboxBackend,
    mirrors: &MirrorsConfig,
    proxy_on: bool,
) -> Result<()> {
    // ---- Alpine APK repositories ----
    let alpine_bases = mirrors.alpine_repo_urls(proxy_on);
    // Detect Alpine version inside the sandbox
    let version_id = backend
        .exec("grep -oP 'VERSION_ID=\\K.*' /etc/os-release 2>/dev/null || echo '3.23'")
        .await
        .unwrap_or_else(|_| "3.23".into());
    let version_id = version_id.trim();
    // Extract major.minor (e.g., "3.23.2" → "3.23")
    let parts: Vec<&str> = version_id.split('.').collect();
    let v_short = if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        version_id.to_string()
    };

    // Build multi-line repositories file: each base contributes main +
    // community lines. apk tries them in order on fetch failure.
    let mut repos = String::new();
    for base in &alpine_bases {
        repos.push_str(&format!("{base}/v{v_short}/main\n"));
        repos.push_str(&format!("{base}/v{v_short}/community\n"));
    }
    // /etc/apk/repositories is root-owned; `limactl shell` default user is
    // clawenv (NOPASSWD sudo available). Stream through `sudo tee` rather
    // than `cat > /etc/...` to avoid Permission denied.
    backend
        .exec(&format!(
            "sudo tee /etc/apk/repositories > /dev/null << 'REPOEOF'\n{repos}REPOEOF"
        ))
        .await?;
    tracing::info!(
        "Alpine APK repositories written: {} base URL(s), v{v_short}",
        alpine_bases.len()
    );

    // ---- npm registry ----
    let npm_candidates = mirrors.npm_registry_urls(proxy_on);
    if let Some(chosen) = select_reachable_npm(backend, &npm_candidates).await {
        // Only set registry when it's different from npm's default —
        // otherwise we noop and let npm use its baked-in upstream.
        if chosen != "https://registry.npmjs.org" {
            backend
                .exec(&format!("npm config set registry '{chosen}'"))
                .await?;
            tracing::info!("npm registry set to {chosen}");
        }
    }

    Ok(())
}

/// HEAD-check each candidate URL via the sandbox's own curl, return the
/// first one that responds 2xx/3xx. We run the check inside the VM so
/// the network path matches what the eventual `npm install` will use
/// (including any host-proxy indirection via `host.lima.internal`).
///
/// Returns None only if every candidate fails — caller then uses the
/// first URL anyway and lets npm surface the real error.
async fn select_reachable_npm(
    backend: &dyn SandboxBackend,
    candidates: &[String],
) -> Option<String> {
    for url in candidates {
        // `curl --head --max-time 5 --silent --fail` returns 0 on 2xx/3xx.
        // -L to follow redirects (npmjs.org 301s to www.npmjs.org).
        let probe = format!(
            "curl -sfLI --max-time 5 --connect-timeout 3 -o /dev/null '{url}/-/ping' && echo ok || echo fail"
        );
        match backend.exec(&probe).await {
            Ok(out) if out.contains("ok") => {
                tracing::info!("npm preflight: {url} reachable");
                return Some(url.clone());
            }
            Ok(_) => {
                tracing::info!("npm preflight: {url} unreachable, trying next");
            }
            Err(e) => {
                tracing::warn!("npm preflight: backend.exec failed ({e}); falling back");
                // If the backend itself can't run curl (no curl installed,
                // backend communication error), abort preflight and let
                // caller use first candidate.
                return candidates.first().cloned();
            }
        }
    }
    // All failed — return first candidate, caller will try to use it and
    // npm will produce a proper error.
    candidates.first().cloned()
}

/// Generate the Alpine repositories content for use in provision scripts
/// (before the sandbox is fully booted, e.g., Lima YAML or WSL provision
/// script). Multi-line — apk fallback applies line by line.
///
/// Returns an empty string only when there are zero URLs (which
/// shouldn't happen — at minimum the upstream dl-cdn URL is in
/// mirrors.toml's `[apk].official_base_urls`).
pub fn alpine_repo_script(
    mirrors: &MirrorsConfig,
    _alpine_version: &str,
    proxy_on: bool,
) -> String {
    let bases = mirrors.alpine_repo_urls(proxy_on);
    if bases.is_empty() {
        return String::new();
    }

    // Detect version at runtime inside the sandbox (works on all Alpine
    // images). Each base contributes main + community lines. Using
    // `>>` after the first so we don't clobber; the initial `>` wipes
    // any prior file.
    let mut out = String::from(
        "ALPINE_VER=$(cat /etc/alpine-release 2>/dev/null | cut -d. -f1,2)\n\
         if [ -n \"$ALPINE_VER\" ]; then\n",
    );
    for (i, base) in bases.iter().enumerate() {
        let redir = if i == 0 { ">" } else { ">>" };
        out.push_str(&format!(
            "  echo '{base}/v'\"$ALPINE_VER\"'/main' {redir} /etc/apk/repositories\n"
        ));
        out.push_str(&format!(
            "  echo '{base}/v'\"$ALPINE_VER\"'/community' >> /etc/apk/repositories\n"
        ));
    }
    out.push_str("fi\n");
    out
}

/// Generate npm registry configuration command for provision scripts.
/// Returns a single `npm config set` line pointing at the first
/// candidate — provision is too early to do a preflight (no curl yet
/// and no sandbox exec loop). Post-boot `apply_mirrors` will do the
/// preflight and potentially switch. If the user has set a concrete
/// override, we use that (preflight doesn't help for hand-configured
/// internal mirrors).
pub fn npm_registry_script(mirrors: &MirrorsConfig, proxy_on: bool) -> String {
    let candidates = mirrors.npm_registry_urls(proxy_on);
    let first = match candidates.first() {
        Some(u) if u != "https://registry.npmjs.org" => u.clone(),
        _ => return String::new(),
    };
    format!("npm config set registry '{first}'\n")
}
