//! Fast go/no-go connectivity probe against a small fixed set of
//! load-bearing hosts. Designed for "should I even try an install?"
//! decisions — much cheaper than `DownloadOps::check_connectivity`,
//! which sweeps every catalog URL.
//!
//! Mirrors v1's `platform/preflight.rs` three-point probe (Alpine CDN,
//! npm, github), with GFW-aware messaging baked into the severity levels.

use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::common::OpsError;

/// The three hosts v1 found to be load-bearing for any install path:
/// Alpine mirror (sandbox package install), npm (dashboard deps),
/// GitHub (dugite / MinGit / Lima / OpenClaw / Hermes releases).
pub const PREFLIGHT_HOSTS: &[&str] =
    &["dl-cdn.alpinelinux.org", "registry.npmjs.org", "github.com"];

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostProbe {
    pub host: String,
    pub reachable: bool,
    pub http_status: Option<u16>,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightReport {
    pub hosts: Vec<HostProbe>,
    /// Rollup: true when every host is reachable. The UI can use this to
    /// decide "should we even start install?".
    pub all_reachable: bool,
    /// Snapshot of the well-known proxy env vars so users can see whether
    /// the failure is likely proxy-misconfiguration vs real network.
    pub http_proxy_env: Option<String>,
    pub https_proxy_env: Option<String>,
    pub no_proxy_env: Option<String>,
    pub suggestion: Option<String>,
    pub checked_at: String,
}

/// Probe the fixed preflight set in parallel and return a structured
/// report. Uses HEAD requests per host; a 2xx/3xx response counts as
/// reachable. Any error — including TLS handshake failures, DNS, or
/// connect timeouts — is captured in the per-host `error` field.
pub async fn run_preflight() -> Result<PreflightReport, OpsError> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .user_agent("clawops-preflight/0.1")
        .build()
        .map_err(|e| OpsError::Other(anyhow::anyhow!("preflight client: {e}")))?;

    let futures = PREFLIGHT_HOSTS.iter().map(|host| {
        let client = client.clone();
        let host_s = host.to_string();
        async move { probe_one(&client, host_s).await }
    });
    let hosts: Vec<HostProbe> = futures_util::future::join_all(futures).await;
    let all_reachable = hosts.iter().all(|h| h.reachable);

    let http_proxy_env = read_env("HTTP_PROXY").or_else(|| read_env("http_proxy"));
    let https_proxy_env = read_env("HTTPS_PROXY").or_else(|| read_env("https_proxy"));
    let no_proxy_env = read_env("NO_PROXY").or_else(|| read_env("no_proxy"));

    let suggestion = build_suggestion(
        all_reachable,
        &hosts,
        http_proxy_env.is_some() || https_proxy_env.is_some(),
    );

    Ok(PreflightReport {
        hosts,
        all_reachable,
        http_proxy_env,
        https_proxy_env,
        no_proxy_env,
        suggestion,
        checked_at: Utc::now().to_rfc3339(),
    })
}

async fn probe_one(client: &reqwest::Client, host: String) -> HostProbe {
    let url = format!("https://{host}/");
    let t0 = Instant::now();
    match client.head(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let reachable = status.is_success() || status.is_redirection();
            HostProbe {
                host,
                reachable,
                http_status: Some(status.as_u16()),
                latency_ms: Some(t0.elapsed().as_millis() as u64),
                error: if reachable {
                    None
                } else {
                    Some(format!("HTTP {}", status.as_u16()))
                },
            }
        }
        Err(e) => HostProbe {
            host,
            reachable: false,
            http_status: None,
            latency_ms: None,
            error: Some(e.to_string()),
        },
    }
}

fn read_env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn build_suggestion(all_reachable: bool, hosts: &[HostProbe], proxy_set: bool) -> Option<String> {
    if all_reachable {
        return None;
    }
    let failed: Vec<&str> = hosts
        .iter()
        .filter(|h| !h.reachable)
        .map(|h| h.host.as_str())
        .collect();
    if proxy_set {
        Some(format!(
            "Cannot reach {}. HTTP(S)_PROXY is set — check that the proxy itself is up and allows these hosts.",
            failed.join(", ")
        ))
    } else {
        Some(format!(
            "Cannot reach {}. Consider enabling an HTTPS proxy (export HTTPS_PROXY=...) before retrying the install.",
            failed.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_hosts_is_nonempty_and_canonical() {
        assert!(!PREFLIGHT_HOSTS.is_empty());
        for h in PREFLIGHT_HOSTS {
            assert!(
                !h.contains("://"),
                "host entries should be bare hostnames, got: {h}"
            );
            assert!(!h.contains('/'), "no paths in host entries, got: {h}");
        }
    }

    #[test]
    fn suggestion_mentions_proxy_when_set() {
        let hosts = vec![HostProbe {
            host: "github.com".into(),
            reachable: false,
            http_status: None,
            latency_ms: None,
            error: Some("timed out".into()),
        }];
        let s = build_suggestion(false, &hosts, true).unwrap();
        assert!(s.contains("HTTP(S)_PROXY is set"));
        assert!(s.contains("github.com"));
    }

    #[test]
    fn suggestion_recommends_proxy_when_unset() {
        let hosts = vec![HostProbe {
            host: "registry.npmjs.org".into(),
            reachable: false,
            http_status: None,
            latency_ms: None,
            error: Some("connect: timed out".into()),
        }];
        let s = build_suggestion(false, &hosts, false).unwrap();
        assert!(s.contains("enabling"));
        assert!(s.contains("registry.npmjs.org"));
    }

    #[test]
    fn suggestion_absent_when_all_reachable() {
        let hosts = vec![HostProbe {
            host: "x".into(),
            reachable: true,
            http_status: Some(200),
            latency_ms: Some(10),
            error: None,
        }];
        assert!(build_suggestion(true, &hosts, false).is_none());
    }
}
