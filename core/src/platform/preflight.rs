//! Host-side connectivity preflight used by install / upgrade / import
//! entry points.
//!
//! The v0.3.0 contract is "connectivity is the user's problem" — that
//! only works if we can *cheaply* tell the user when their problem has
//! arrived. This module probes the three canonical endpoints the install
//! pipeline will hit (npm registry, github, nodejs.org) and reports
//! which ones are reachable under the active proxy triple.
//!
//! Callers:
//!   - `clawcli install/upgrade/import` run this at command entry, bail
//!     if any endpoint fails. Gives CLI users the same gate the GUI
//!     wizard's StepNetwork gives GUI users.
//!   - `tauri::ipc::network::test_connectivity` wraps this with
//!     per-endpoint event emission for the StepNetwork UI.
//!
//! Intentionally *not* doing fallback-tier selection, proxy detection,
//! or DNS warm-up — we just answer "can this proxy reach these URLs?"
//! and surface the raw verdict. Anything fancier lives in the caller.

use anyhow::Result;
use serde::Serialize;

/// One endpoint's probe outcome.
#[derive(Debug, Clone, Serialize)]
pub struct PreflightResult {
    pub endpoint: String,
    pub url: String,
    pub ok: bool,
    pub message: String,
}

/// The canonical endpoint list. Each covers a distinct failure mode:
/// npm (usually Cloudflare-CDN reachable), github (classic blocked
/// target), nodejs.org (node dist downloads), dl-cdn (Alpine apk).
pub fn canonical_endpoints() -> Vec<(&'static str, &'static str)> {
    vec![
        ("npm Registry", "https://registry.npmjs.org/"),
        ("GitHub", "https://api.github.com/"),
        ("Node.js dist", "https://nodejs.org/dist/"),
        ("Alpine CDN", "https://dl-cdn.alpinelinux.org/alpine/latest-stable/"),
    ]
}

/// Run the preflight probe set using `proxy_url` (None = use system
/// default, Some("") = explicit no-proxy, Some(url) = use that proxy).
/// Returns one result per endpoint in the canonical order.
pub async fn run(proxy_url: Option<&str>) -> Result<Vec<PreflightResult>> {
    run_with_endpoints(proxy_url, &canonical_endpoints()).await
}

/// Same as `run` but lets the caller pass a custom endpoint list.
/// Used by the Tauri wrapper so it can pass through whatever the
/// StepNetwork UI decided was relevant (including user-customised
/// mirror overrides).
pub async fn run_with_endpoints(
    proxy_url: Option<&str>,
    endpoints: &[(&str, &str)],
) -> Result<Vec<PreflightResult>> {
    run_with_callback(proxy_url, endpoints, |_| {}).await
}

/// Like `run_with_endpoints` but invokes `cb` once per endpoint as soon
/// as that endpoint's verdict is known — supports per-endpoint progress
/// streaming in the UI (matches StepNetwork's one-at-a-time render).
///
/// The callback receives the `PreflightResult` by reference so it can
/// emit a log line / progress event without copying. The result is still
/// collected into the returned Vec for `bail_if_unreachable`.
pub async fn run_with_callback<F>(
    proxy_url: Option<&str>,
    endpoints: &[(&str, &str)],
    mut cb: F,
) -> Result<Vec<PreflightResult>>
where
    F: FnMut(&PreflightResult),
{
    let client = build_client(proxy_url)?;
    let mut results = Vec::with_capacity(endpoints.len());
    for (name, url) in endpoints {
        let res = probe_one(&client, url).await;
        let r = PreflightResult {
            endpoint: (*name).to_string(),
            url: (*url).to_string(),
            ok: res.0,
            message: res.1,
        };
        cb(&r);
        results.push(r);
    }
    Ok(results)
}

fn build_client(proxy_url: Option<&str>) -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10));
    let client = match proxy_url {
        // Some("") = explicit no-proxy — disable reqwest's auto env-proxy
        // pickup so we're honestly testing direct.
        Some("") => builder.no_proxy().build()?,
        Some(url) => {
            let proxy = reqwest::Proxy::all(url)?;
            builder.proxy(proxy).no_proxy().build()?
        }
        // None = use reqwest defaults (env-proxy pickup if set).
        None => builder.build()?,
    };
    Ok(client)
}

async fn probe_one(client: &reqwest::Client, url: &str) -> (bool, String) {
    let res = client
        .get(url)
        .header("User-Agent", "ClawEnv/0.3")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    match res {
        Ok(r) if r.status().is_success() || r.status().is_redirection() => {
            (true, format!("OK ({})", r.status()))
        }
        Ok(r) => (false, format!("HTTP {}", r.status())),
        Err(e) => {
            let err_str = e.to_string();
            let msg = if err_str.contains("timed out") {
                "Timeout (15s)".into()
            } else if err_str.contains("dns") || err_str.contains("resolve") {
                "DNS resolution failed".into()
            } else if err_str.contains("connect") {
                "Connection refused".into()
            } else {
                err_str
            };
            (false, msg)
        }
    }
}

/// Convenience: fail loudly if any endpoint probe returned `ok=false`.
/// The error message is bilingual and lists the failing endpoints.
/// Used by CLI `install/upgrade/import` as a pre-command gate —
/// mirrors the StepNetwork gate on the GUI side.
pub fn bail_if_unreachable(results: &[PreflightResult]) -> Result<()> {
    let failures: Vec<String> = results.iter()
        .filter(|r| !r.ok)
        .map(|r| format!("  - {}: {}", r.endpoint, r.message))
        .collect();
    if !failures.is_empty() {
        anyhow::bail!(
            "连通性预检未通过，请先解决网络问题再重试。\n\
             Connectivity preflight failed — fix your network before retrying.\n\n\
             失败端点 / Failed endpoints:\n{}",
            failures.join("\n")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_endpoints_cover_three_distinct_hosts() {
        // Regression guard for the "single-endpoint false positive"
        // problem: npm-Cloudflare-reachable doesn't mean github or
        // nodejs.org are reachable. The canonical list must include
        // hosts that have historically had independent failure modes.
        let eps = canonical_endpoints();
        let hosts: std::collections::HashSet<&str> = eps.iter()
            .filter_map(|(_, url)| url.split('/').nth(2))
            .collect();
        assert!(hosts.contains("registry.npmjs.org"), "npm host missing");
        assert!(hosts.contains("api.github.com"), "github host missing");
        assert!(hosts.contains("nodejs.org"), "nodejs.org host missing");
        // At least 3 distinct hosts so no single-host outage disguises
        // a multi-host problem.
        assert!(hosts.len() >= 3, "need at least 3 distinct hosts, got {}", hosts.len());
    }

    #[test]
    fn bail_if_unreachable_passes_on_all_ok() {
        let results = vec![
            PreflightResult {
                endpoint: "npm".into(), url: "https://registry.npmjs.org/".into(),
                ok: true, message: "OK (200)".into(),
            },
            PreflightResult {
                endpoint: "github".into(), url: "https://api.github.com/".into(),
                ok: true, message: "OK (200)".into(),
            },
        ];
        assert!(bail_if_unreachable(&results).is_ok());
    }

    #[test]
    fn bail_if_unreachable_fails_on_any_bad() {
        let results = vec![
            PreflightResult {
                endpoint: "npm".into(), url: "https://registry.npmjs.org/".into(),
                ok: true, message: "OK (200)".into(),
            },
            PreflightResult {
                endpoint: "github".into(), url: "https://api.github.com/".into(),
                ok: false, message: "Timeout (15s)".into(),
            },
        ];
        let err = bail_if_unreachable(&results).unwrap_err();
        let msg = err.to_string();
        // Bilingual + names the failing endpoint.
        assert!(msg.contains("Connectivity preflight failed"), "english marker missing: {msg}");
        assert!(msg.contains("连通性预检"), "chinese marker missing: {msg}");
        assert!(msg.contains("github"), "failing endpoint name missing: {msg}");
    }

    #[test]
    fn bail_if_unreachable_on_empty_results_passes() {
        // Empty input means "nothing to check" — not an error per se.
        // Caller's responsibility to ensure the probe actually ran.
        assert!(bail_if_unreachable(&[]).is_ok());
    }
}
