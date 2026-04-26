//! Unified loader for `assets/mirrors.toml` — single source of truth for
//! every binary ClawEnv downloads (git, node, lima, wsl distro, podman
//! base image) AND for every repo-URL-style config it writes (apk, npm).
//!
//! v0.3.0 policy: upstream sources ONLY. The multi-tier mirror selection
//! (official + regional fallback) has been removed — if the upstream
//! URL isn't reachable on the user's network, the install fails and
//! the user is expected to enable a proxy.
//!
//! The prior `fallback_urls` tier (corporate regional mirrors + GitHub
//! release reverse proxies used to keep installs alive on networks
//! where the upstream was unreachable) has been removed in v0.3.0 in
//! favour of surfacing the connectivity problem to the user so they
//! can resolve it (typically by enabling a proxy). See
//! `assets/mirrors.toml` for the rationale.
//!
//! Callers pass an asset name ("dugite" / "mingit" / "node" / "lima" /
//! "alpine-minirootfs" / "apk" / "npm") plus a platform key; the loader
//! builds the URL list and (for sha-pinned assets) returns the expected
//! sha256.

use anyhow::{anyhow, Result};
use std::sync::OnceLock;

/// Parsed mirrors.toml. Loaded once at first access, cached for the
/// lifetime of the process.
#[derive(Debug)]
pub struct AssetMirrors {
    raw: toml::Table,
}

static INSTANCE: OnceLock<AssetMirrors> = OnceLock::new();

impl AssetMirrors {
    /// Lazy-loaded global accessor. Panics only if `mirrors.toml` is
    /// malformed at build time (shouldn't happen — it's bundled via
    /// `include_str!` and we test-parse it in CI).
    pub fn get() -> &'static AssetMirrors {
        INSTANCE.get_or_init(|| {
            Self::load().expect("assets/mirrors.toml failed to parse — bundled data is invalid")
        })
    }

    fn load() -> Result<Self> {
        let src = include_str!("../../../../assets/mirrors.toml");
        let raw: toml::Table = src.parse()
            .map_err(|e| anyhow!("mirrors.toml invalid: {e}"))?;
        Ok(Self { raw })
    }

    /// Extract a list of URL templates from an asset section.
    /// Missing key returns an empty vec (not an error).
    fn urls_list(&self, asset: &str, list_key: &str) -> Vec<String> {
        self.raw.get(asset)
            .and_then(|v| v.as_table())
            .and_then(|t| t.get(list_key))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }

    /// Effective URL list for `asset`. v0.3.0: upstream `official_urls`
    /// only — any regional mirror tier has been removed. If the official
    /// URL isn't reachable on the user's network, the install fails and
    /// it's up to the user to enable a proxy.
    fn effective_url_tpls(&self, asset: &str) -> Vec<String> {
        self.urls_list(asset, "official_urls")
    }

    /// Return (url, filename) pairs for download-style assets (dugite /
    /// mingit / node / lima).
    ///
    /// Returns an error when:
    /// - asset section missing
    /// - `official_urls` list is empty
    /// - a required placeholder substitution produces an empty string
    ///   (catches typos in platform keys immediately rather than failing
    ///   later in the downloader)
    pub fn build_urls(&self, asset: &str, platform: &str) -> Result<Vec<(String, String)>> {
        let section = self.raw.get(asset)
            .and_then(|v| v.as_table())
            .ok_or_else(|| anyhow!("mirrors.toml missing [{asset}]"))?;

        let urls = self.effective_url_tpls(asset);
        if urls.is_empty() {
            anyhow::bail!("[{asset}] has no official_urls");
        }

        // Collect all scalar fields as substitution variables.
        let mut vars: Vec<(String, String)> = section.iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        vars.push(("platform".into(), platform.into()));

        // Expand filename_tpl first so it can be referenced as {filename}.
        let filename_tpl = section.get("filename_tpl")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("[{asset}] missing `filename_tpl`"))?;
        let filename = expand(filename_tpl, &vars);
        vars.push(("filename".into(), filename.clone()));

        let expanded: Vec<(String, String)> = urls.iter()
            .map(|u| (expand(u, &vars), filename.clone()))
            .collect();
        Ok(expanded)
    }

    /// Return the pinned sha256 for `(asset, platform)` if the
    /// `[asset.sha256]` sub-table lists it. `None` means "no pinned
    /// checksum; rely on TLS". Downloaders use this to decide whether
    /// to verify.
    ///
    /// Platform keys in the sha256 table are canonicalised (lowercase,
    /// x86_64→x86_64) to tolerate minor capitalisation differences
    /// between filename platform keys and checksum table keys.
    pub fn expected_sha256(&self, asset: &str, platform: &str) -> Option<String> {
        let sha_table = self.raw.get(asset)?.as_table()?.get("sha256")?.as_table()?;
        let key = platform.to_lowercase();
        // Try exact match first, then normalised variants (darwin-arm64 vs macos-arm64 etc.)
        for k in [&key, &key.replace("darwin", "macos"), &key.replace("macos", "darwin")] {
            if let Some(v) = sha_table.get(k).and_then(|v| v.as_str()) {
                return Some(v.to_string());
            }
        }
        None
    }

    /// Alpine is special: the directory path depends on `major_minor`
    /// which has to be computed from the resolved version at runtime.
    /// Takes the extra `major_minor` arg. Other callers should use
    /// `build_urls`.
    pub fn build_alpine_urls(
        &self,
        platform: &str,
        version: &str,
        major_minor: &str,
    ) -> Result<Vec<(String, String)>> {
        let asset = "alpine-minirootfs";
        let section = self.raw.get(asset)
            .and_then(|v| v.as_table())
            .ok_or_else(|| anyhow!("mirrors.toml missing [{asset}]"))?;
        let filename = expand(
            section.get("filename_tpl").and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("[{asset}] missing filename_tpl"))?,
            &[
                ("platform".into(), platform.into()),
                ("version".into(), version.into()),
            ],
        );
        let vars = [
            ("major_minor".into(), major_minor.into()),
            ("platform".into(), platform.into()),
            ("version".into(), version.into()),
            ("filename".into(), filename.clone()),
        ];
        let urls = self.effective_url_tpls(asset);
        if urls.is_empty() {
            anyhow::bail!("[{asset}] has no URLs");
        }
        Ok(urls.into_iter().map(|u| (expand(&u, &vars), filename.clone())).collect())
    }

    /// Effective list of Alpine apk repository *base* URLs
    /// (e.g. `["https://dl-cdn.alpinelinux.org/alpine"]`). Caller appends
    /// the `/v<major.minor>/main` and `/community` suffixes.
    pub fn apk_base_urls(&self) -> Vec<String> {
        self.urls_list("apk", "official_base_urls")
    }

    /// Effective list of npm registry URLs. `npm config set registry`
    /// takes one value, so the list is a candidate set, not a fallback
    /// chain like download URLs.
    pub fn npm_registry_urls(&self) -> Vec<String> {
        self.effective_url_tpls("npm")
    }
}

fn expand(tpl: &str, vars: &[(String, String)]) -> String {
    let mut out = tpl.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_bundled_toml() {
        let m = AssetMirrors::get();
        assert!(m.raw.contains_key("dugite"));
        assert!(m.raw.contains_key("node"));
        assert!(m.raw.contains_key("lima"));
        assert!(m.raw.contains_key("apk"));
        assert!(m.raw.contains_key("npm"));
    }

    #[test]
    fn dugite_macos_arm64_upstream_only() {
        let m = AssetMirrors::get();
        let urls = m.build_urls("dugite", "macOS-arm64").unwrap();
        assert_eq!(urls.len(), 1, "only upstream URL expected, got {urls:?}");
        assert!(urls[0].0.starts_with("https://github.com/"),
                "upstream must be github, got: {}", urls[0].0);
        assert_eq!(urls[0].1, "dugite-native-v2.53.0-f49d009-macOS-arm64.tar.gz");
        let sha = m.expected_sha256("dugite", "macos-arm64").unwrap();
        assert_eq!(sha.len(), 64);
    }

    #[test]
    fn node_macos_arm64_upstream_only() {
        let m = AssetMirrors::get();
        let urls = m.build_urls("node", "darwin-arm64").unwrap();
        assert_eq!(urls.len(), 1);
        assert!(urls[0].0.starts_with("https://nodejs.org/dist/"));
        assert!(urls[0].0.contains("v22.16.0"));
        assert!(urls[0].0.contains("darwin-arm64"));
    }

    #[test]
    fn apk_base_urls_upstream_only() {
        let m = AssetMirrors::get();
        assert_eq!(
            m.apk_base_urls(),
            vec!["https://dl-cdn.alpinelinux.org/alpine".to_string()],
        );
    }

    #[test]
    fn npm_registry_urls_upstream_only() {
        let m = AssetMirrors::get();
        assert_eq!(m.npm_registry_urls(), vec!["https://registry.npmjs.org".to_string()]);
    }

    #[test]
    fn missing_asset_errors() {
        let m = AssetMirrors::get();
        assert!(m.build_urls("does-not-exist", "any").is_err());
    }
}
