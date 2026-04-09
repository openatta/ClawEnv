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

/// Check npm registry for the latest version of OpenClaw.
pub async fn check_latest_version(current_version: &str) -> Result<VersionInfo> {
    let client = reqwest::Client::builder()
        .user_agent("ClawEnv/0.1")
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let resp = client
        .get("https://registry.npmjs.org/openclaw/latest")
        .send()
        .await?;

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

    // Clean current version: "OpenClaw 2026.4.5 (abc123)" → "2026.4.5"
    let current_clean = current_version
        .trim()
        .trim_start_matches("OpenClaw ")
        .split_whitespace()
        .next()
        .unwrap_or(current_version)
        .trim_start_matches('v')
        .to_string();

    let has_upgrade = if let (Ok(cur), Ok(lat)) = (
        semver::Version::parse(&current_clean),
        semver::Version::parse(&latest),
    ) {
        lat > cur
    } else {
        latest != current_clean
    };

    let is_security = pkg.description.to_lowercase().contains("cve")
        || pkg.description.to_lowercase().contains("security");

    Ok(VersionInfo {
        current: current_clean,
        latest,
        changelog: pkg.description,
        is_security_release: is_security,
        has_upgrade,
    })
}
