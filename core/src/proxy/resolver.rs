//! Scope-driven proxy resolution.
//!
//! v1's `proxy_resolver::Scope` has three variants; this port keeps
//! the same semantics but defers OS-level system detection. What we
//! cover today:
//!
//! - [`Scope::Installer`] — host-side downloads during install/upgrade.
//!   Checks shell env vars, then config.
//! - [`Scope::RuntimeNative`] — child processes spawned on the host.
//!   Policy: env only (config on disk doesn't apply to native runs).
//! - [`Scope::RuntimeSandbox`] — commands that will run inside a VM.
//!   Checks the per-instance override, then global config, then env;
//!   whichever we pick gets its loopback addresses rewritten so the
//!   VM can reach the host.
//!
//! Loopback rewriting is the subtle bit: `http://127.0.0.1:7890` on
//! the host is unreachable from inside Lima/WSL/Podman; the three
//! backends each have a "host bridge" hostname we translate to.

use crate::sandbox_ops::BackendKind;

use super::config::{
    InstanceProxyConfig, InstanceProxyMode, ProxyConfig, ProxySource, ProxyTriple,
};

/// Where a resolved proxy will be used. The three variants gate
/// different policy around precedence and loopback rewriting.
pub enum Scope<'a> {
    /// Host-side downloads during install/upgrade. Env > Config.
    Installer,
    /// Host-side runtime (native claws). Env only — no config.
    RuntimeNative,
    /// Sandbox-side runtime. Per-instance > global config > env,
    /// with loopback rewriting applied so the VM can reach the host.
    RuntimeSandbox {
        backend: BackendKind,
        instance: Option<&'a InstanceProxyConfig>,
    },
}

impl Scope<'_> {
    /// Resolve the active proxy triple for this scope, if any.
    ///
    /// `global` is the `[clawenv.proxy]` section. Most callers will
    /// pass `&ProxyConfig::default()` when no global config is loaded
    /// yet (v2 doesn't own config.toml loading yet).
    ///
    /// For [`Scope::RuntimeSandbox`] we need an asynchronous lookup
    /// when `backend == Wsl2` (we exec `cat /etc/resolv.conf` inside
    /// the VM to get the nameserver for loopback rewriting). Everything
    /// else is a sync computation but we keep the signature async for
    /// uniformity.
    pub async fn resolve(
        &self,
        global: &ProxyConfig,
        wsl_nameserver: Option<&str>,
    ) -> Option<ProxyTriple> {
        match self {
            Scope::Installer => resolve_installer(global),
            Scope::RuntimeNative => read_env_triple(),
            Scope::RuntimeSandbox { backend, instance } => {
                resolve_runtime_sandbox(global, *backend, *instance, wsl_nameserver)
            }
        }
    }
}

fn resolve_installer(global: &ProxyConfig) -> Option<ProxyTriple> {
    // Env takes precedence: users running `HTTPS_PROXY=... clawops ...`
    // expect that to override whatever's on disk.
    if let Some(t) = read_env_triple() {
        return Some(t);
    }
    triple_from_config(global, ProxySource::GlobalConfig)
}

fn resolve_runtime_sandbox(
    global: &ProxyConfig,
    backend: BackendKind,
    instance: Option<&InstanceProxyConfig>,
    wsl_nameserver: Option<&str>,
) -> Option<ProxyTriple> {
    // Per-instance override wins first.
    let host_addr = sandbox_host_address(backend, wsl_nameserver);
    if let Some(i) = instance {
        match i.mode {
            InstanceProxyMode::None => return None,
            InstanceProxyMode::Manual => {
                if !i.http_proxy.is_empty() || !i.https_proxy.is_empty() {
                    return Some(ProxyTriple {
                        http: rewrite_loopback(&i.http_proxy, &host_addr),
                        https: rewrite_loopback(&i.https_proxy, &host_addr),
                        no_proxy: if i.no_proxy.is_empty() {
                            default_no_proxy().into()
                        } else {
                            i.no_proxy.clone()
                        },
                        source: ProxySource::PerInstance,
                    });
                }
                // Manual but empty — fall through to global.
            }
            InstanceProxyMode::SyncHost => {
                // Fall through to global config; only loopback rewriting
                // differs from the `None` branch.
            }
        }
    }
    // Global config, rewritten.
    if let Some(t) = triple_from_config(global, ProxySource::GlobalConfig) {
        return Some(ProxyTriple {
            http: rewrite_loopback(&t.http, &host_addr),
            https: rewrite_loopback(&t.https, &host_addr),
            no_proxy: t.no_proxy,
            source: t.source,
        });
    }
    // Final fallback: env.
    let env = read_env_triple()?;
    Some(ProxyTriple {
        http: rewrite_loopback(&env.http, &host_addr),
        https: rewrite_loopback(&env.https, &host_addr),
        no_proxy: env.no_proxy,
        source: ProxySource::ShellEnv,
    })
}

fn triple_from_config(cfg: &ProxyConfig, src: ProxySource) -> Option<ProxyTriple> {
    if !cfg.enabled {
        return None;
    }
    let http = super::url::proxy_url_with_auth(cfg).ok()?;
    let https = if cfg.https_proxy.is_empty() {
        http.clone()
    } else {
        // For auth_required with different https_proxy, we reuse the
        // auth-composition; if auth isn't required, http_proxy==https_proxy
        // we can just clone.
        let mut with_https = cfg.clone();
        with_https.http_proxy = cfg.https_proxy.clone();
        super::url::proxy_url_with_auth(&with_https).ok()?
    };
    let no_proxy = if cfg.no_proxy.is_empty() {
        default_no_proxy().into()
    } else {
        cfg.no_proxy.clone()
    };
    if http.is_empty() && https.is_empty() {
        return None;
    }
    Some(ProxyTriple {
        http,
        https,
        no_proxy,
        source: src,
    })
}

fn read_env_triple() -> Option<ProxyTriple> {
    let http = env_case_insensitive("HTTP_PROXY")?;
    let https = env_case_insensitive("HTTPS_PROXY").unwrap_or_else(|| http.clone());
    let no_proxy =
        env_case_insensitive("NO_PROXY").unwrap_or_else(|| default_no_proxy().into());
    Some(ProxyTriple {
        http,
        https,
        no_proxy,
        source: ProxySource::ShellEnv,
    })
}

fn env_case_insensitive(k: &str) -> Option<String> {
    std::env::var(k)
        .ok()
        .or_else(|| std::env::var(k.to_lowercase()).ok())
        .filter(|v| !v.is_empty())
}

fn default_no_proxy() -> &'static str { "localhost,127.0.0.1" }

/// Host bridge hostname for each sandbox backend. Same table as v1:
///
/// - Lima   → `host.lima.internal`
/// - Podman → `host.containers.internal`
/// - WSL2   → nameserver IP from `/etc/resolv.conf` inside the VM;
///   callers pass it as `wsl_nameserver`; fallback to
///   `host.docker.internal` (usually correct on Win11 WSL2 with
///   mirrored networking).
pub fn sandbox_host_address(backend: BackendKind, wsl_nameserver: Option<&str>) -> String {
    match backend {
        BackendKind::Lima => "host.lima.internal".into(),
        BackendKind::Podman => "host.containers.internal".into(),
        BackendKind::Wsl2 => wsl_nameserver
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "host.docker.internal".into()),
    }
}

/// Replace loopback hostnames in `url` with `new_host`. No-op when
/// `new_host == 127.0.0.1` (native scope) or when the URL contains no
/// loopback segment at all.
pub fn rewrite_loopback(url: &str, new_host: &str) -> String {
    if url.is_empty() { return String::new(); }
    if new_host == "127.0.0.1" { return url.to_string(); }
    url.replace("127.0.0.1", new_host)
        .replace("://localhost", &format!("://{new_host}"))
}

#[cfg(test)]
// The test mutex is held across .await to serialize env-var mutation.
// No async code inside these tests actually blocks on anything that
// might want the same mutex, so the deadlock risk clippy worries
// about doesn't apply.
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Every test in this module reads env vars; serialize them.
    static LOCK: Mutex<()> = Mutex::new(());

    fn clean_env() -> std::sync::MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        for k in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy"] {
            unsafe { std::env::remove_var(k); }
        }
        g
    }

    #[test]
    fn rewrite_loopback_replaces_127() {
        assert_eq!(
            rewrite_loopback("http://127.0.0.1:7890", "host.lima.internal"),
            "http://host.lima.internal:7890"
        );
    }

    #[test]
    fn rewrite_loopback_replaces_localhost() {
        assert_eq!(
            rewrite_loopback("http://localhost:7890", "host.lima.internal"),
            "http://host.lima.internal:7890"
        );
    }

    #[test]
    fn rewrite_loopback_noop_for_native_host() {
        assert_eq!(
            rewrite_loopback("http://127.0.0.1:7890", "127.0.0.1"),
            "http://127.0.0.1:7890"
        );
    }

    #[test]
    fn rewrite_loopback_passes_non_loopback_through() {
        assert_eq!(
            rewrite_loopback("http://192.168.1.10:8080", "host.lima.internal"),
            "http://192.168.1.10:8080"
        );
        assert_eq!(
            rewrite_loopback("http://proxy.corp:3128", "host.lima.internal"),
            "http://proxy.corp:3128"
        );
    }

    #[test]
    fn rewrite_loopback_empty_is_empty() {
        assert_eq!(rewrite_loopback("", "host.lima.internal"), "");
    }

    #[test]
    fn sandbox_host_lima_and_podman_are_static() {
        assert_eq!(sandbox_host_address(BackendKind::Lima, None), "host.lima.internal");
        assert_eq!(sandbox_host_address(BackendKind::Podman, None), "host.containers.internal");
    }

    #[test]
    fn sandbox_host_wsl_uses_nameserver_when_given() {
        assert_eq!(
            sandbox_host_address(BackendKind::Wsl2, Some("172.26.64.1")),
            "172.26.64.1"
        );
    }

    #[test]
    fn sandbox_host_wsl_falls_back_when_empty() {
        assert_eq!(sandbox_host_address(BackendKind::Wsl2, None), "host.docker.internal");
        assert_eq!(sandbox_host_address(BackendKind::Wsl2, Some("")), "host.docker.internal");
    }

    #[test]
    fn read_env_returns_none_when_clean() {
        let _g = clean_env();
        assert!(read_env_triple().is_none());
    }

    #[test]
    fn read_env_uppercase_wins() {
        let _g = clean_env();
        unsafe {
            std::env::set_var("HTTP_PROXY", "http://upper:1");
            std::env::set_var("http_proxy", "http://lower:2");
        }
        let t = read_env_triple().unwrap();
        assert_eq!(t.http, "http://upper:1");
    }

    #[test]
    fn read_env_falls_back_to_lowercase() {
        let _g = clean_env();
        unsafe { std::env::set_var("http_proxy", "http://lc:1"); }
        let t = read_env_triple().unwrap();
        assert_eq!(t.http, "http://lc:1");
    }

    #[test]
    fn read_env_https_defaults_to_http_when_missing() {
        let _g = clean_env();
        unsafe { std::env::set_var("HTTP_PROXY", "http://x:1"); }
        let t = read_env_triple().unwrap();
        assert_eq!(t.https, "http://x:1");
    }

    #[test]
    fn read_env_no_proxy_defaults_when_missing() {
        let _g = clean_env();
        unsafe { std::env::set_var("HTTP_PROXY", "http://x:1"); }
        let t = read_env_triple().unwrap();
        assert_eq!(t.no_proxy, "localhost,127.0.0.1");
    }

    #[test]
    fn triple_from_config_disabled_is_none() {
        let cfg = ProxyConfig { enabled: false, ..Default::default() };
        assert!(triple_from_config(&cfg, ProxySource::GlobalConfig).is_none());
    }

    #[test]
    fn triple_from_config_enabled_populates() {
        let cfg = ProxyConfig {
            enabled: true,
            http_proxy: "http://proxy:3128".into(),
            ..Default::default()
        };
        let t = triple_from_config(&cfg, ProxySource::GlobalConfig).unwrap();
        assert_eq!(t.http, "http://proxy:3128");
        assert_eq!(t.https, "http://proxy:3128"); // mirrored
        assert_eq!(t.no_proxy, "localhost,127.0.0.1");
    }

    #[tokio::test]
    async fn installer_env_beats_config() {
        let _g = clean_env();
        unsafe { std::env::set_var("HTTP_PROXY", "http://env:1"); }
        let cfg = ProxyConfig {
            enabled: true,
            http_proxy: "http://cfg:2".into(),
            ..Default::default()
        };
        let t = Scope::Installer.resolve(&cfg, None).await.unwrap();
        assert_eq!(t.http, "http://env:1");
        assert_eq!(t.source, ProxySource::ShellEnv);
    }

    #[tokio::test]
    async fn installer_falls_back_to_config() {
        let _g = clean_env();
        let cfg = ProxyConfig {
            enabled: true,
            http_proxy: "http://cfg:2".into(),
            ..Default::default()
        };
        let t = Scope::Installer.resolve(&cfg, None).await.unwrap();
        assert_eq!(t.http, "http://cfg:2");
        assert_eq!(t.source, ProxySource::GlobalConfig);
    }

    #[tokio::test]
    async fn sandbox_none_mode_suppresses_global_proxy() {
        let _g = clean_env();
        let cfg = ProxyConfig {
            enabled: true,
            http_proxy: "http://cfg:2".into(),
            ..Default::default()
        };
        let i = InstanceProxyConfig { mode: InstanceProxyMode::None, ..Default::default() };
        let scope = Scope::RuntimeSandbox { backend: BackendKind::Lima, instance: Some(&i) };
        assert!(scope.resolve(&cfg, None).await.is_none());
    }

    #[tokio::test]
    async fn sandbox_manual_mode_uses_instance_config_with_rewrite() {
        let _g = clean_env();
        let cfg = ProxyConfig::default();
        let i = InstanceProxyConfig {
            mode: InstanceProxyMode::Manual,
            http_proxy: "http://127.0.0.1:7890".into(),
            https_proxy: String::new(),
            no_proxy: String::new(),
        };
        let scope = Scope::RuntimeSandbox { backend: BackendKind::Lima, instance: Some(&i) };
        let t = scope.resolve(&cfg, None).await.unwrap();
        assert_eq!(t.http, "http://host.lima.internal:7890");
        assert_eq!(t.source, ProxySource::PerInstance);
    }

    #[tokio::test]
    async fn sandbox_sync_host_rewrites_global_loopback() {
        let _g = clean_env();
        let cfg = ProxyConfig {
            enabled: true,
            http_proxy: "http://127.0.0.1:7890".into(),
            ..Default::default()
        };
        let i = InstanceProxyConfig {
            mode: InstanceProxyMode::SyncHost, ..Default::default()
        };
        let scope = Scope::RuntimeSandbox { backend: BackendKind::Podman, instance: Some(&i) };
        let t = scope.resolve(&cfg, None).await.unwrap();
        assert_eq!(t.http, "http://host.containers.internal:7890");
        assert_eq!(t.source, ProxySource::GlobalConfig);
    }

    #[tokio::test]
    async fn sandbox_wsl_uses_nameserver_for_rewrite() {
        let _g = clean_env();
        let cfg = ProxyConfig {
            enabled: true,
            http_proxy: "http://127.0.0.1:7890".into(),
            ..Default::default()
        };
        let scope = Scope::RuntimeSandbox { backend: BackendKind::Wsl2, instance: None };
        let t = scope.resolve(&cfg, Some("172.26.64.1")).await.unwrap();
        assert_eq!(t.http, "http://172.26.64.1:7890");
    }

    #[tokio::test]
    async fn runtime_native_ignores_config_uses_env_only() {
        let _g = clean_env();
        let cfg = ProxyConfig {
            enabled: true,
            http_proxy: "http://cfg:2".into(),
            ..Default::default()
        };
        // No env set: even though config enabled, native scope skips it.
        assert!(Scope::RuntimeNative.resolve(&cfg, None).await.is_none());

        unsafe { std::env::set_var("HTTP_PROXY", "http://env:1"); }
        let t = Scope::RuntimeNative.resolve(&cfg, None).await.unwrap();
        assert_eq!(t.http, "http://env:1");
    }
}
