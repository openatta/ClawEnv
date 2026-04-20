//! Unified loader for `assets/mirrors.toml` — single source of truth for
//! every binary ClawEnv downloads (git, node, lima, wsl distro, podman
//! base image) AND for every repo-URL-style config it writes (apk, npm).
//!
//! Two-tier proxy-aware model (v0.2.14+):
//!
//!   official_urls  — upstream, authoritative. Always included.
//!   fallback_urls  — corporate domestic mirrors (aliyun / huaweicloud /
//!                    npmmirror / ghfast.top). Appended ONLY when the
//!                    caller passes `proxy_on=false`.
//!
//! Callers pass an asset name ("dugite" / "mingit" / "node" / "lima" /
//! "alpine-minirootfs" / "apk" / "npm") plus a platform key; the loader
//! builds the fallback URL list and (for sha-pinned assets) returns the
//! expected sha256. On-disk grammar is documented in `assets/mirrors.toml`.

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

    /// Extract a list of URL templates from an asset section. `list_key`
    /// is one of "official_urls" / "fallback_urls" / "official_base_urls"
    /// / "fallback_base_urls". Missing keys return an empty vec (not an
    /// error — most assets have one tier but not both, and apk uses the
    /// `_base_urls` variants).
    fn urls_list(&self, asset: &str, list_key: &str) -> Vec<String> {
        self.raw.get(asset)
            .and_then(|v| v.as_table())
            .and_then(|t| t.get(list_key))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default()
    }

    /// Assemble the effective URL list for `asset` based on the install-time
    /// proxy snapshot.
    ///
    /// proxy ON  → official_urls only. Proxy is expected to give clean
    ///             access to upstream; domestic CN mirrors are pointless
    ///             detours and (in the SJTU case) actively break.
    /// proxy OFF → fallback_urls FIRST, then official_urls. The user has
    ///             chosen to install without a proxy, which in practice
    ///             means they're either (a) on an unrestricted network
    ///             where official upstream works fine and CN mirrors are
    ///             still fast enough to lead, or (b) inside the GFW
    ///             where official upstream trickles to a halt and the
    ///             CN mirror is the only viable path. Leading with the
    ///             fast CN mirror covers both cases — official upstream
    ///             is appended as a last-resort safety net.
    fn effective_url_tpls(&self, asset: &str, proxy_on: bool) -> Vec<String> {
        if proxy_on {
            self.urls_list(asset, "official_urls")
        } else {
            let mut out = self.urls_list(asset, "fallback_urls");
            out.extend(self.urls_list(asset, "official_urls"));
            out
        }
    }

    /// Return (url, filename) pairs in fallback order, for download-style
    /// assets (dugite/mingit/node/lima). `proxy_on` controls whether
    /// fallback_urls are appended.
    ///
    /// Returns an error when:
    /// - asset section missing
    /// - both URL tiers empty
    /// - a required placeholder substitution produces an empty string
    ///   (catches typos in platform keys immediately rather than failing
    ///   later in the downloader)
    pub fn build_urls(&self, asset: &str, platform: &str, proxy_on: bool) -> Result<Vec<(String, String)>> {
        let section = self.raw.get(asset)
            .and_then(|v| v.as_table())
            .ok_or_else(|| anyhow!("mirrors.toml missing [{asset}]"))?;

        let urls = self.effective_url_tpls(asset, proxy_on);
        if urls.is_empty() {
            anyhow::bail!("[{asset}] has no URLs (both official_urls and fallback_urls empty)");
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
        proxy_on: bool,
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
        let urls = self.effective_url_tpls(asset, proxy_on);
        if urls.is_empty() {
            anyhow::bail!("[{asset}] has no URLs");
        }
        Ok(urls.into_iter().map(|u| (expand(&u, &vars), filename.clone())).collect())
    }

    /// Return the effective list of Alpine apk repository *base* URLs
    /// (e.g. `["https://dl-cdn.alpinelinux.org/alpine",
    /// "https://mirrors.aliyun.com/alpine"]`). Caller appends the
    /// `/v<major.minor>/main` and `/community` suffixes. Uses the
    /// `[apk].official_base_urls` + `fallback_base_urls` scheme rather
    /// than the `urls`/`filename` download pattern — apk consumes the
    /// bases directly, no filename placeholder.
    pub fn apk_base_urls(&self, proxy_on: bool) -> Vec<String> {
        // Same policy as effective_url_tpls — proxy off leads with CN mirror.
        // apk's `/etc/apk/repositories` reads top-down so first-listed wins
        // for the initial fetch; subsequent ones are tried on per-package
        // failures. With aliyun first, GFW users see ~1.5 MB/s instead of
        // dl-cdn's trickle.
        if proxy_on {
            self.urls_list("apk", "official_base_urls")
        } else {
            let mut out = self.urls_list("apk", "fallback_base_urls");
            out.extend(self.urls_list("apk", "official_base_urls"));
            out
        }
    }

    /// Effective list of npm registry URLs. Caller picks the first
    /// reachable one via a preflight HEAD. `npm config set registry`
    /// takes one value, so the list is a candidate set, not a fallback
    /// chain like download URLs.
    pub fn npm_registry_urls(&self, proxy_on: bool) -> Vec<String> {
        self.effective_url_tpls("npm", proxy_on)
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
    fn dugite_macos_arm64_expands_proxy_off() {
        // proxy off → fallback (GH relay) leads, official upstream appended.
        let m = AssetMirrors::get();
        let urls = m.build_urls("dugite", "macOS-arm64", false).unwrap();
        assert!(urls.len() >= 2, "proxy off → fallback + official");
        // First URL is a GH-relay fallback (not raw github.com).
        assert!(!urls[0].0.starts_with("https://github.com/"),
                "fallback should lead, got: {}", urls[0].0);
        assert!(urls.iter().any(|(u, _)| u.starts_with("https://github.com/")),
                "official upstream must be present as safety net");
        assert_eq!(urls[0].1, "dugite-native-v2.53.0-f49d009-macOS-arm64.tar.gz");
        let sha = m.expected_sha256("dugite", "macos-arm64").unwrap();
        assert_eq!(sha.len(), 64);
    }

    #[test]
    fn dugite_macos_arm64_expands_proxy_on() {
        let m = AssetMirrors::get();
        let urls = m.build_urls("dugite", "macOS-arm64", true).unwrap();
        assert_eq!(urls.len(), 1, "proxy on → official only");
        assert!(urls[0].0.starts_with("https://github.com/"));
    }

    #[test]
    fn node_macos_arm64_expands() {
        let m = AssetMirrors::get();
        let urls = m.build_urls("node", "darwin-arm64", false).unwrap();
        assert!(urls.len() >= 2);
        assert!(urls[0].0.contains("v22.16.0"));
        assert!(urls[0].0.contains("darwin-arm64"));
    }

    #[test]
    fn apk_base_urls_proxy_on_only_official() {
        let m = AssetMirrors::get();
        assert_eq!(m.apk_base_urls(true), vec!["https://dl-cdn.alpinelinux.org/alpine".to_string()]);
    }

    #[test]
    fn apk_base_urls_proxy_off_includes_fallback() {
        // Per v0.2.13 policy flip: proxy off → fallback (CN mirror) leads,
        // official upstream is appended last as safety net. apk reads
        // /etc/apk/repositories top-down, so CN mirror gets first attempt.
        let m = AssetMirrors::get();
        let urls = m.apk_base_urls(false);
        assert!(urls.len() >= 2);
        assert!(urls[0].contains("aliyun"), "fallback (CN mirror) should lead, got: {}", urls[0]);
        assert!(urls.iter().any(|u| u == "https://dl-cdn.alpinelinux.org/alpine"));
    }

    #[test]
    fn npm_registry_urls_have_both_tiers_off() {
        // Same policy: npmmirror leads when proxy off, official npmjs is
        // appended as safety net.
        let m = AssetMirrors::get();
        let urls = m.npm_registry_urls(false);
        assert!(urls.len() >= 2);
        assert!(urls[0].contains("npmmirror"), "fallback should lead, got: {}", urls[0]);
        assert!(urls.iter().any(|u| u == "https://registry.npmjs.org"));
    }

    #[test]
    fn missing_asset_errors() {
        let m = AssetMirrors::get();
        assert!(m.build_urls("does-not-exist", "any", false).is_err());
    }
}
