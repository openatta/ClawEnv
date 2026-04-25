//! Latest-version probes for npm / PyPI / GitHub-released claws.
//!
//! Lifted verbatim from v1 `core/src/update/checker.rs` (P1-b). This
//! module is **pure HTTP via reqwest** — no v1-specific types, no
//! sandbox interaction. The lift required zero functional changes;
//! only the doc header is new.
//!
//! Used by:
//! - `clawcli upgrade --check` (CLI-side gate before doing the install)
//! - Tauri main.rs background loop (every 5 min, surface "upgrade
//!   available" badges in the UI)
//! - `clawcli upgrade <name>` orchestrator (resolve "latest" to a
//!   concrete version when the user passes `--to latest`)

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub current: String,
    pub latest: String,
    pub changelog: String,
    pub is_security_release: bool,
    pub has_upgrade: bool,
}

/// Build a reusable HTTP client for version checks.
fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("ClawEnv/0.2")
        .timeout(std::time::Duration::from_secs(10))
        .build()?)
}

/// Clean version string: "OpenClaw 2026.4.5 (abc123)" → "2026.4.5", "Hermes Agent 0.3.0" → "0.3.0"
fn clean_version(raw: &str) -> String {
    // Strip known product prefixes
    let stripped = raw.trim()
        .trim_start_matches("OpenClaw ")
        .trim_start_matches("Hermes Agent ")
        .trim_start_matches("hermes-agent ");
    stripped
        .split_whitespace()
        .next()
        .unwrap_or(raw)
        .trim_start_matches('v')
        .to_string()
}

/// Compare two version strings and determine if latest > current.
fn has_upgrade(current: &str, latest: &str) -> bool {
    if let (Ok(cur), Ok(lat)) = (
        semver::Version::parse(current),
        semver::Version::parse(latest),
    ) {
        lat > cur
    } else {
        latest != current
    }
}

// ---- npm registry ----

/// Check npm registry for the latest version of a claw package.
/// `npm_registry` defaults to "https://registry.npmjs.org" if empty.
pub async fn check_latest_npm(current_version: &str, npm_registry: &str, npm_package: &str) -> Result<VersionInfo> {
    let registry = if npm_registry.is_empty() { "https://registry.npmjs.org" } else { npm_registry };
    let package = if npm_package.is_empty() { "openclaw" } else { npm_package };
    let url = format!("{}/{}/latest", registry.trim_end_matches('/'), package);

    let client = http_client()?;
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("npm registry returned {}", resp.status());
    }

    #[derive(Deserialize)]
    struct NpmPkg {
        version: String,
        #[serde(default)]
        description: String,
    }

    let pkg: NpmPkg = resp.json().await?;
    let latest = pkg.version.trim().to_string();
    let current_clean = clean_version(current_version);

    let is_security = pkg.description.to_lowercase().contains("cve")
        || pkg.description.to_lowercase().contains("security");

    Ok(VersionInfo {
        has_upgrade: has_upgrade(&current_clean, &latest),
        current: current_clean,
        latest,
        changelog: pkg.description,
        is_security_release: is_security,
    })
}

// ---- PyPI registry ----

/// Check PyPI for the latest version of a pip package.
/// Uses the JSON API: https://pypi.org/pypi/<package>/json
pub async fn check_latest_pypi(current_version: &str, pip_package: &str) -> Result<VersionInfo> {
    if pip_package.is_empty() {
        anyhow::bail!("pip_package is empty — cannot check PyPI");
    }
    let url = format!("https://pypi.org/pypi/{}/json", pip_package);

    let client = http_client()?;
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("PyPI returned {} for package '{}'", resp.status(), pip_package);
    }

    #[derive(Deserialize)]
    struct PyPiInfo {
        version: String,
        #[serde(default)]
        summary: String,
    }
    #[derive(Deserialize)]
    struct PyPiResponse {
        info: PyPiInfo,
    }

    let data: PyPiResponse = resp.json().await?;
    let latest = data.info.version.trim().to_string();
    let current_clean = clean_version(current_version);

    let is_security = data.info.summary.to_lowercase().contains("cve")
        || data.info.summary.to_lowercase().contains("security");

    Ok(VersionInfo {
        has_upgrade: has_upgrade(&current_clean, &latest),
        current: current_clean,
        latest,
        changelog: data.info.summary,
        is_security_release: is_security,
    })
}

// ---- GitHub releases ----

/// Check GitHub releases for the latest version of a git-based package.
/// Uses the GitHub API: https://api.github.com/repos/<owner>/<repo>/releases/latest
/// `git_repo` should be a full HTTPS URL like "https://github.com/NousResearch/hermes-agent.git"
pub async fn check_latest_github(current_version: &str, git_repo: &str) -> Result<VersionInfo> {
    // Extract owner/repo from URL: "https://github.com/NousResearch/hermes-agent.git" → "NousResearch/hermes-agent"
    let repo_path = git_repo
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .rsplit("github.com/")
        .next()
        .ok_or_else(|| anyhow::anyhow!("Cannot parse GitHub repo from: {git_repo}"))?;

    let url = format!("https://api.github.com/repos/{repo_path}/releases/latest");

    let client = http_client()?;
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("GitHub API returned {} for '{repo_path}'", resp.status());
    }

    #[derive(Deserialize)]
    struct GhRelease {
        tag_name: String,
        #[serde(default)]
        body: String,
    }

    let release: GhRelease = resp.json().await?;
    let latest = release.tag_name.trim().trim_start_matches('v').to_string();
    let current_clean = clean_version(current_version);

    let is_security = release.body.to_lowercase().contains("cve")
        || release.body.to_lowercase().contains("security");

    Ok(VersionInfo {
        has_upgrade: has_upgrade(&current_clean, &latest),
        current: current_clean,
        latest,
        changelog: release.body.lines().take(5).collect::<Vec<_>>().join("\n"),
        is_security_release: is_security,
    })
}

/// Backwards-compatible alias: dispatches to npm by default.
/// Prefer `check_latest_npm()` or `check_latest_pypi()` directly.
pub async fn check_latest_version(current_version: &str, npm_registry: &str, npm_package: &str) -> Result<VersionInfo> {
    check_latest_npm(current_version, npm_registry, npm_package).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_version_openclaw() {
        assert_eq!(clean_version("OpenClaw 2026.4.5 (abc123)"), "2026.4.5");
        assert_eq!(clean_version("v1.2.3"), "1.2.3");
    }

    #[test]
    fn test_clean_version_hermes() {
        assert_eq!(clean_version("Hermes Agent 0.3.0"), "0.3.0");
        assert_eq!(clean_version("hermes-agent 0.3.0"), "0.3.0");
        assert_eq!(clean_version("0.3.0"), "0.3.0");
    }

    #[test]
    fn test_has_upgrade_semver() {
        assert!(has_upgrade("1.0.0", "1.0.1"));
        assert!(has_upgrade("1.0.0", "2.0.0"));
        assert!(!has_upgrade("1.0.1", "1.0.0"));
        assert!(!has_upgrade("1.0.0", "1.0.0"));
    }

    #[test]
    fn test_has_upgrade_non_semver() {
        // Fallback: string comparison
        assert!(has_upgrade("2024.1", "2024.2"));
        assert!(!has_upgrade("same", "same"));
    }
}
