//! Read the global `~/.clawenv/config.toml` written by v1, extracting
//! the parts v2 needs today: `[clawenv.proxy]` and `[clawenv.mirrors]`.
//!
//! Serde's default "accept unknown fields, fill missing with Default"
//! behaviour means v1 can keep adding sections (bridge, tray, ...)
//! without breaking v2 loads. Conversely v2 doesn't WRITE config.toml
//! yet — that lands with v4 migration work — so this is a one-way
//! read-only bridge.
//!
//! Paths:
//! - Default location: `clawenv_root().join("config.toml")`. Honors
//!   CLAWENV_HOME env so tests don't step on the user's real config.
//! - Missing file: returns a pure-default [`GlobalConfig`] silently
//!   (an absent config is a valid state — first-run users, or users
//!   who haven't enabled a proxy).
//! - Unparseable file: returns `Err` with the path + toml error so
//!   callers can surface it to the CLI.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::paths::clawenv_root;
use crate::provisioning::MirrorsConfig;
use crate::proxy::ProxyConfig;

/// What v2 cares about from the global config.toml. Everything else
/// (tray, bridge, instances, etc.) is silently ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalConfig {
    pub proxy: ProxyConfig,
    pub mirrors: MirrorsConfig,
}

/// Default location v1 writes its config.toml to. CLAWENV_HOME env
/// var overrides the root.
pub fn default_config_path() -> PathBuf {
    clawenv_root().join("config.toml")
}

#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("I/O error reading {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },

    #[error("toml parse error in {path}: {source}")]
    Parse { path: PathBuf, #[source] source: toml::de::Error },
}

/// Load the global config from v1's canonical path. Missing file is
/// NOT an error — returns `Default`.
pub fn load_global() -> Result<GlobalConfig, ConfigLoadError> {
    load_from_path(&default_config_path())
}

/// Load from a specific path. Missing file yields Default; anything
/// else surfaces the specific failure.
pub fn load_from_path(p: &Path) -> Result<GlobalConfig, ConfigLoadError> {
    let bytes = match std::fs::read_to_string(p) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GlobalConfig::default());
        }
        Err(source) => {
            return Err(ConfigLoadError::Io { path: p.to_path_buf(), source });
        }
    };
    parse_toml_str(&bytes).map_err(|source| ConfigLoadError::Parse {
        path: p.to_path_buf(),
        source,
    })
}

// ——— Parse helpers ———

fn parse_toml_str(s: &str) -> Result<GlobalConfig, toml::de::Error> {
    // Wrapper structs mirror v1's layout. Fields we don't care about
    // are absent here — serde silently ignores them in the source toml.
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct V1Root {
        clawenv: V1ClawEnv,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct V1ClawEnv {
        proxy: ProxyConfig,
        mirrors: MirrorsConfig,
    }
    let root: V1Root = toml::from_str(s)?;
    Ok(GlobalConfig {
        proxy: root.clawenv.proxy,
        mirrors: root.clawenv.mirrors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_file_yields_default() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nonexistent.toml");
        let g = load_from_path(&missing).unwrap();
        assert_eq!(g, GlobalConfig::default());
    }

    #[test]
    fn empty_file_yields_default() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("empty.toml");
        std::fs::write(&p, "").unwrap();
        let g = load_from_path(&p).unwrap();
        assert_eq!(g, GlobalConfig::default());
    }

    #[test]
    fn empty_clawenv_section_yields_default() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("c.toml");
        std::fs::write(&p, "[clawenv]\n").unwrap();
        let g = load_from_path(&p).unwrap();
        assert_eq!(g, GlobalConfig::default());
    }

    #[test]
    fn extracts_proxy_from_v1_shape() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("c.toml");
        std::fs::write(&p, r#"
[clawenv.proxy]
enabled = true
http_proxy = "http://corp:3128"
https_proxy = "http://corp:3128"
no_proxy = "localhost,127.0.0.1"
auth_required = false
auth_user = ""
"#).unwrap();
        let g = load_from_path(&p).unwrap();
        assert!(g.proxy.enabled);
        assert_eq!(g.proxy.http_proxy, "http://corp:3128");
        assert_eq!(g.proxy.no_proxy, "localhost,127.0.0.1");
    }

    #[test]
    fn extracts_mirrors_from_v1_shape() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("c.toml");
        std::fs::write(&p, r#"
[clawenv.mirrors]
alpine_repo = "https://mirrors.aliyun.com/alpine"
npm_registry = "https://registry.npmmirror.com"
"#).unwrap();
        let g = load_from_path(&p).unwrap();
        assert_eq!(g.mirrors.alpine_repo, "https://mirrors.aliyun.com/alpine");
        assert_eq!(g.mirrors.npm_registry, "https://registry.npmmirror.com");
    }

    #[test]
    fn ignores_fields_v2_does_not_know_about() {
        // v1 has lots of sections — tray, bridge, updates, etc.
        // Plus stray "nodejs_dist" in mirrors that v2 dropped. Must not
        // fail to parse.
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("c.toml");
        std::fs::write(&p, r#"
[clawenv]
version = "0.3.2"
user_mode = "general"
language = "en"

[clawenv.tray]
enabled = true

[clawenv.proxy]
enabled = true
http_proxy = "http://corp:3128"

[clawenv.mirrors]
alpine_repo = "https://mirrors.aliyun.com/alpine"
npm_registry = ""
nodejs_dist = "https://npmmirror.com/mirrors/node"

[clawenv.bridge]
mode = "disabled"

[[instances]]
name = "foo"
"#).unwrap();
        let g = load_from_path(&p).unwrap();
        assert!(g.proxy.enabled);
        assert_eq!(g.proxy.http_proxy, "http://corp:3128");
        assert_eq!(g.mirrors.alpine_repo, "https://mirrors.aliyun.com/alpine");
        // nodejs_dist silently ignored; npm_registry defaulted.
        assert_eq!(g.mirrors.npm_registry, "");
    }

    #[test]
    fn malformed_toml_surfaces_parse_error_with_path() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("bad.toml");
        std::fs::write(&p, "not = [valid toml").unwrap();
        let err = load_from_path(&p).unwrap_err();
        match err {
            ConfigLoadError::Parse { path, .. } => {
                assert_eq!(path, p);
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }
}
