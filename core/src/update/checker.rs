use anyhow::Result;
use serde::{Deserialize, Serialize};

const GITHUB_RELEASES_URL: &str =
    "https://api.github.com/repos/openclaw/openclaw/releases/latest";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub current: String,
    pub latest: String,
    pub changelog: String,
    pub is_security_release: bool,
    pub download_url: Option<String>,
}

impl VersionInfo {
    pub fn has_upgrade(&self) -> bool {
        if let (Ok(current), Ok(latest)) = (
            semver::Version::parse(self.current.trim_start_matches('v')),
            semver::Version::parse(self.latest.trim_start_matches('v')),
        ) {
            latest > current
        } else {
            false
        }
    }
}

/// Check GitHub Releases for a newer version of OpenClaw
pub async fn check_latest_version(current_version: &str) -> Result<VersionInfo> {
    let client = reqwest::Client::builder()
        .user_agent("ClawEnv/0.1")
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let resp = client.get(GITHUB_RELEASES_URL).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("GitHub API returned {}", resp.status());
    }

    let release: GithubRelease = resp.json().await?;

    let is_security = release.body.to_lowercase().contains("cve")
        || release.body.to_lowercase().contains("security");

    Ok(VersionInfo {
        current: current_version.to_string(),
        latest: release.tag_name.trim_start_matches('v').to_string(),
        changelog: release.body,
        is_security_release: is_security,
        download_url: release.assets.first().map(|a| a.browser_download_url.clone()),
    })
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    body: String,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    browser_download_url: String,
}
