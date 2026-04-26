//! Proxy data model. Mirrors v1's `config::models::{ProxyConfig,
//! InstanceProxyConfig}` so a config.toml written by v1 can be read
//! (and vice versa) once v2 grows toml loading.

use serde::{Deserialize, Serialize};

/// Global proxy preference, serialised under `[clawenv.proxy]` in v1.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProxyConfig {
    pub enabled: bool,
    pub http_proxy: String,
    pub https_proxy: String,
    /// Comma-separated hosts (v1 default: `"localhost,127.0.0.1"`).
    pub no_proxy: String,
    pub auth_required: bool,
    /// Username only; password lives in the credentials vault.
    pub auth_user: String,
}

/// Per-instance override.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct InstanceProxyConfig {
    pub mode: InstanceProxyMode,
    pub http_proxy: String,
    pub https_proxy: String,
    pub no_proxy: String,
}

/// How an instance selects its proxy (v1 contract).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InstanceProxyMode {
    /// Explicitly no proxy inside this sandbox, regardless of global config.
    #[default]
    None,
    /// Use this instance's own `http_proxy` / `https_proxy` / `no_proxy`.
    Manual,
    /// Follow the global [`ProxyConfig`], rewriting loopback for the
    /// sandbox's host bridge.
    SyncHost,
}

/// A resolved triple ready to be applied somewhere. `source` is
/// purely informational — useful for `clawops proxy resolve --json`
/// so the user can see WHY a value was chosen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyTriple {
    pub http: String,
    pub https: String,
    pub no_proxy: String,
    pub source: ProxySource,
}

impl ProxyTriple {
    /// True when every URL field is empty (nothing to apply).
    pub fn is_empty(&self) -> bool {
        self.http.is_empty() && self.https.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProxySource {
    /// Loaded from `[clawenv.proxy]` in the global config.
    GlobalConfig,
    /// Loaded from `[instances.<name>.proxy]` for this instance.
    PerInstance,
    /// Read from `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` env vars.
    ShellEnv,
    /// Detected from the host OS (deferred; not yet implemented).
    OsSystem,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_config_defaults() {
        let p = ProxyConfig::default();
        assert!(!p.enabled);
        assert_eq!(p.http_proxy, "");
        assert_eq!(p.auth_user, "");
    }

    #[test]
    fn instance_proxy_mode_default_is_none() {
        assert_eq!(InstanceProxyMode::default(), InstanceProxyMode::None);
    }

    #[test]
    fn triple_is_empty_when_urls_are_empty() {
        let t = ProxyTriple {
            http: String::new(),
            https: String::new(),
            no_proxy: "localhost".into(),
            source: ProxySource::GlobalConfig,
        };
        assert!(t.is_empty());
    }

    #[test]
    fn triple_roundtrips_json() {
        let t = ProxyTriple {
            http: "http://proxy:3128".into(),
            https: "http://proxy:3128".into(),
            no_proxy: "localhost,127.0.0.1".into(),
            source: ProxySource::ShellEnv,
        };
        let j = serde_json::to_string(&t).unwrap();
        let back: ProxyTriple = serde_json::from_str(&j).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn proxy_source_serializes_kebab_case() {
        let j = serde_json::to_string(&ProxySource::OsSystem).unwrap();
        assert_eq!(j, "\"os-system\"");
        let j = serde_json::to_string(&ProxySource::GlobalConfig).unwrap();
        assert_eq!(j, "\"global-config\"");
    }

    #[test]
    fn instance_mode_kebab_case_serialization() {
        assert_eq!(serde_json::to_string(&InstanceProxyMode::SyncHost).unwrap(), "\"sync-host\"");
        assert_eq!(serde_json::to_string(&InstanceProxyMode::None).unwrap(), "\"none\"");
    }
}
