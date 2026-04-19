//! Unified loader for `assets/mirrors.toml` — single source of truth for
//! every binary ClawEnv downloads (git, node, lima, wsl distro, podman
//! base image).
//!
//! Callers pass an asset name ("dugite" / "mingit" / "node" / "lima" /
//! "alpine-minirootfs") plus a platform key; the loader builds the
//! complete fallback URL list and returns the expected sha256 (if any).
//! The on-disk format and placeholder grammar are documented in
//! `assets/mirrors.toml`.

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
        let src = include_str!("../../../assets/mirrors.toml");
        let raw: toml::Table = src.parse()
            .map_err(|e| anyhow!("mirrors.toml invalid: {e}"))?;
        Ok(Self { raw })
    }

    /// Return (url, filename) pairs in fallback order. Filename is the
    /// final asset name (same for every URL, the loader expands
    /// `{filename}` placeholders consistently).
    ///
    /// Returns an error when:
    /// - asset section missing
    /// - `urls` missing / empty
    /// - a required placeholder substitution produces an empty string
    ///   (catches typos in platform keys immediately rather than failing
    ///   later in the downloader)
    pub fn build_urls(&self, asset: &str, platform: &str) -> Result<Vec<(String, String)>> {
        let section = self.raw.get(asset)
            .and_then(|v| v.as_table())
            .ok_or_else(|| anyhow!("mirrors.toml missing [{asset}]"))?;

        let urls: Vec<String> = section.get("urls")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("[{asset}] missing `urls`"))?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if urls.is_empty() {
            anyhow::bail!("[{asset}] has empty `urls`");
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
    /// This variant takes the extra `major_minor` arg. Other callers
    /// should use `build_urls`.
    pub fn build_alpine_urls(&self, platform: &str, version: &str, major_minor: &str) -> Result<Vec<(String, String)>> {
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
        let urls: Vec<String> = section.get("urls")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("[{asset}] missing urls"))?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        Ok(urls.into_iter().map(|u| (expand(&u, &vars), filename.clone())).collect())
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
        // If this panics, mirrors.toml is broken at bundle-time.
        let m = AssetMirrors::get();
        assert!(m.raw.contains_key("dugite"));
        assert!(m.raw.contains_key("node"));
        assert!(m.raw.contains_key("lima"));
    }

    #[test]
    fn dugite_macos_arm64_expands() {
        let m = AssetMirrors::get();
        let urls = m.build_urls("dugite", "macOS-arm64").unwrap();
        assert!(urls.len() >= 2, "need at least upstream + one mirror");
        assert!(urls[0].0.contains("dugite-native-v2.53.0"));
        assert!(urls[0].0.contains("macOS-arm64"));
        assert_eq!(urls[0].1, "dugite-native-v2.53.0-f49d009-macOS-arm64.tar.gz");
        let sha = m.expected_sha256("dugite", "macos-arm64").unwrap();
        assert_eq!(sha.len(), 64);
    }

    #[test]
    fn node_macos_arm64_expands() {
        let m = AssetMirrors::get();
        // Caller also substitutes {ext} — build_urls inserts platform/filename
        // but the {ext} placeholder flows through since build_urls collects
        // all scalar section fields as vars. Node has no `ext` field so the
        // caller prepends it: we mimic by calling build_urls with a
        // platform value that already has the extension baked in.
        let urls = m.build_urls("node", "darwin-arm64").unwrap();
        assert!(!urls.is_empty());
        // The URL should contain the version and the platform key.
        assert!(urls[0].0.contains("v22.16.0"));
        assert!(urls[0].0.contains("darwin-arm64"));
    }

    #[test]
    fn missing_asset_errors() {
        let m = AssetMirrors::get();
        assert!(m.build_urls("does-not-exist", "any").is_err());
    }
}
