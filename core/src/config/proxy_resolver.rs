//! Unified proxy resolver — single source of truth for every piece of
//! code that needs "what proxy should I use right now?".
//!
//! Design: every caller declares its **scope** (Installer / RuntimeNative
//! / RuntimeSandbox). The resolver walks a scope-specific priority chain
//! and returns an `Option<ProxyTriple>` with a `source` field attached for
//! debugging. See `docs/23-proxy-architecture.md` for the full spec.
//!
//! Scopes in a nutshell:
//! - `Installer`        → host downloads (shell env → config → OS detect → direct)
//! - `RuntimeNative`    → native claw (OS detect only, by design)
//! - `RuntimeSandbox`   → sandbox claw (per-VM config → global config → OS detect → direct),
//!                         with 127.0.0.1/localhost rewritten to host.lima.internal etc.

use anyhow::Result;

use super::manager::ConfigManager;
use super::models::{InstanceConfig, ProxyConfig};
use super::proxy::proxy_url_with_auth;
use crate::sandbox::{SandboxBackend, SandboxType};

/// Fully resolved proxy values. Callers don't need to touch env vars,
/// config fields, or keychain themselves — they just feed this into
/// `apply_env` / `apply_child_cmd` / `apply_to_sandbox`.
#[derive(Debug, Clone)]
pub struct ProxyTriple {
    pub http: String,
    pub https: String,
    pub no_proxy: String,
    pub source: ProxySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxySource {
    /// `InstanceConfig.proxy` — per-VM override set via SandboxPage/VmCard.
    PerVm,
    /// `config.toml.clawenv.proxy` — global explicit user choice.
    GlobalConfig,
    /// macOS SCDynamicStore / Windows registry / GNOME gsettings.
    OsSystem,
    /// `HTTPS_PROXY` / `HTTP_PROXY` already in the parent process's env
    /// — usually means the user started us from a terminal that had
    /// proxy env set, or a downstream spawn that already inherited one.
    ShellEnv,
}

/// Scope of the query — determines the priority chain.
pub enum Scope<'a> {
    /// Host-side downloads during install/upgrade. Highest
    /// priority is parent env (so dev runs from terminal keep working);
    /// then explicit config; then OS-detected system proxy; else direct.
    Installer,

    /// Native claw process. **System proxy only** — no per-instance
    /// override, no config override. See `docs/23-proxy-architecture.md` §3.
    RuntimeNative,

    /// Sandbox claw process running inside a VM. Needs the backend to
    /// look up the host-reachable address for URL rewriting
    /// (host.lima.internal / host.containers.internal / WSL nameserver).
    RuntimeSandbox {
        instance: &'a InstanceConfig,
        backend: &'a dyn SandboxBackend,
    },
}

impl<'a> Scope<'a> {
    /// Async because `RuntimeSandbox` may need to exec into the VM
    /// (WSL2 resolv.conf) to learn its host-side address.
    pub async fn resolve(&self, cfg: &ConfigManager) -> Option<ProxyTriple> {
        match self {
            Scope::Installer => resolve_installer(cfg),
            Scope::RuntimeNative => resolve_runtime_native(),
            Scope::RuntimeSandbox { instance, backend } => {
                resolve_runtime_sandbox(cfg, instance, *backend).await
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-scope resolution logic. Each is narrowly documented.
// ---------------------------------------------------------------------------

fn resolve_installer(cfg: &ConfigManager) -> Option<ProxyTriple> {
    // 1. Parent env (shell env or spawn-inherited).
    if let Some(t) = read_env_triple(ProxySource::ShellEnv) {
        trace_resolved(&t, "Installer");
        return Some(t);
    }
    // 2. Explicit global config.
    if let Some(t) = triple_from_config_proxy(&cfg.config().clawenv.proxy, ProxySource::GlobalConfig) {
        trace_resolved(&t, "Installer");
        return Some(t);
    }
    // 3. OS detection (blocking; fine for installer which is off the hot path).
    if let Some(t) = detect_os_triple() {
        trace_resolved(&t, "Installer");
        return Some(t);
    }
    None
}

fn resolve_runtime_native() -> Option<ProxyTriple> {
    // Bypass env/config — native policy is strictly OS-detected.
    // (Env exists because it was injected by *us* from OS detection,
    // but we re-query to stay consistent with policy.)
    if let Some(t) = detect_os_triple() {
        trace_resolved(&t, "RuntimeNative");
        return Some(t);
    }
    // If env is set but OS says nothing, trust env (user started us
    // from a terminal that had proxy). This is the dev path.
    if let Some(t) = read_env_triple(ProxySource::ShellEnv) {
        trace_resolved(&t, "RuntimeNative");
        return Some(t);
    }
    None
}

async fn resolve_runtime_sandbox(
    cfg: &ConfigManager,
    instance: &InstanceConfig,
    backend: &dyn SandboxBackend,
) -> Option<ProxyTriple> {
    // 1. Per-VM override takes absolute priority.
    if let Some(ipc) = instance.proxy.as_ref() {
        match ipc.mode.as_str() {
            "none" => return None, // explicit direct for this VM
            "manual" if !ipc.http_proxy.is_empty() => {
                let http = embed_auth_for_instance(&ipc.http_proxy, ipc, &instance.name);
                let https_raw = if ipc.https_proxy.is_empty() { ipc.http_proxy.clone() } else { ipc.https_proxy.clone() };
                let https = embed_auth_for_instance(&https_raw, ipc, &instance.name);
                let t = ProxyTriple {
                    http,
                    https,
                    no_proxy: if ipc.no_proxy.is_empty() { "localhost,127.0.0.1".into() } else { ipc.no_proxy.clone() },
                    source: ProxySource::PerVm,
                };
                trace_resolved(&t, &format!("RuntimeSandbox[{}]", instance.name));
                return Some(t);
            }
            "sync-host" if !ipc.http_proxy.is_empty() => {
                // Rewrite 127.0.0.1/localhost → backend host address at apply time.
                let host = sandbox_host_address(backend, instance.sandbox_type).await.ok()?;
                let http_auth = embed_auth_for_instance(&ipc.http_proxy, ipc, &instance.name);
                let http = rewrite_loopback(&http_auth, &host);
                let https_raw = if ipc.https_proxy.is_empty() { ipc.http_proxy.clone() } else { ipc.https_proxy.clone() };
                let https_auth = embed_auth_for_instance(&https_raw, ipc, &instance.name);
                let https = rewrite_loopback(&https_auth, &host);
                let t = ProxyTriple {
                    http,
                    https,
                    no_proxy: if ipc.no_proxy.is_empty() { "localhost,127.0.0.1".into() } else { ipc.no_proxy.clone() },
                    source: ProxySource::PerVm,
                };
                trace_resolved(&t, &format!("RuntimeSandbox[{}]", instance.name));
                return Some(t);
            }
            _ => {} // fall through to lower priorities
        }
    }

    // 2. Global config as fallback (needs host translation too).
    if let Some(mut t) = triple_from_config_proxy(&cfg.config().clawenv.proxy, ProxySource::GlobalConfig) {
        if let Ok(host) = sandbox_host_address(backend, instance.sandbox_type).await {
            t.http = rewrite_loopback(&t.http, &host);
            t.https = rewrite_loopback(&t.https, &host);
        }
        trace_resolved(&t, &format!("RuntimeSandbox[{}]", instance.name));
        return Some(t);
    }

    // 3. OS detection as final fallback.
    if let Some(mut t) = detect_os_triple() {
        if let Ok(host) = sandbox_host_address(backend, instance.sandbox_type).await {
            t.http = rewrite_loopback(&t.http, &host);
            t.https = rewrite_loopback(&t.https, &host);
        }
        trace_resolved(&t, &format!("RuntimeSandbox[{}]", instance.name));
        return Some(t);
    }

    None
}

// ---------------------------------------------------------------------------
// Priority-chain building blocks. Each returns an Option<ProxyTriple> or None
// when its layer has nothing to offer. Separated so unit tests can target
// one layer at a time.
// ---------------------------------------------------------------------------

/// Read HTTPS_PROXY / HTTP_PROXY / NO_PROXY from the current process env.
pub(super) fn read_env_triple(source: ProxySource) -> Option<ProxyTriple> {
    let http = std::env::var("HTTP_PROXY").ok()
        .or_else(|| std::env::var("http_proxy").ok())
        .filter(|s| !s.is_empty());
    let https = std::env::var("HTTPS_PROXY").ok()
        .or_else(|| std::env::var("https_proxy").ok())
        .filter(|s| !s.is_empty());
    if http.is_none() && https.is_none() {
        return None;
    }
    let http = http.clone().or_else(|| https.clone()).unwrap_or_default();
    let https = https.unwrap_or_else(|| http.clone());
    let no_proxy = std::env::var("NO_PROXY").ok()
        .or_else(|| std::env::var("no_proxy").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost,127.0.0.1".into());
    Some(ProxyTriple { http, https, no_proxy, source })
}

/// Convert a `ProxyConfig` (with keychain password lookup if auth is on)
/// into a triple. Returns `None` when the config is disabled/empty.
pub fn triple_from_config_proxy(p: &ProxyConfig, source: ProxySource) -> Option<ProxyTriple> {
    if !p.enabled || p.http_proxy.is_empty() {
        return None;
    }
    let http = proxy_url_with_auth(p).unwrap_or_else(|_| p.http_proxy.clone());
    let https = if p.https_proxy.is_empty() { http.clone() } else { p.https_proxy.clone() };
    let no_proxy = if p.no_proxy.is_empty() {
        "localhost,127.0.0.1".into()
    } else {
        p.no_proxy.clone()
    };
    Some(ProxyTriple { http, https, no_proxy, source })
}

/// OS-level detection (macOS SCDynamicStore / Windows registry / Linux
/// gsettings). Returns `None` when no HTTP/HTTPS proxy is set.
///
/// The actual platform-specific detection lives in the `tauri/src/ipc`
/// layer because it uses GUI-only deps. `core` uses a lightweight fallback
/// (env-only) when called from CLI contexts — and a Tauri hook injects
/// the detection result into env at GUI startup, so the CLI subprocess
/// transparently sees it here. See `docs/23-proxy-architecture.md` §10.
pub(super) fn detect_os_triple() -> Option<ProxyTriple> {
    // In `core` we can only see env. If env has values, they were either
    // set by the shell, or injected by the Tauri GUI at startup from its
    // OS detection. Either way they're what "the user wanted" right now.
    read_env_triple(ProxySource::OsSystem)
}

// ---------------------------------------------------------------------------
// Host-side helpers — used only by RuntimeSandbox.
// ---------------------------------------------------------------------------

/// Compute the backend-specific address a VM uses to reach its host.
/// Exposed so `apply_to_sandbox` can use it too.
pub async fn sandbox_host_address(
    backend: &dyn SandboxBackend,
    sandbox_type: SandboxType,
) -> Result<String> {
    match sandbox_type {
        SandboxType::LimaAlpine => Ok("host.lima.internal".into()),
        SandboxType::PodmanAlpine => Ok("host.containers.internal".into()),
        SandboxType::Wsl2Alpine => {
            let out = backend
                .exec("grep -oE '^nameserver[[:space:]]+[0-9.]+' /etc/resolv.conf | head -1 | awk '{print $2}'")
                .await
                .unwrap_or_default();
            let ip = out.trim().to_string();
            Ok(if ip.is_empty() { "host.docker.internal".into() } else { ip })
        }
        SandboxType::Native => Ok("127.0.0.1".into()),
    }
}

/// Embed `user:password@` into a proxy URL using the per-instance
/// keychain entry. If `auth_required` is false or the keychain lookup
/// fails, returns the URL unchanged. Falls back silently — missing
/// password shouldn't block the install, the user can re-enter via the
/// ProxyModal.
fn embed_auth_for_instance(url: &str, ipc: &super::models::InstanceProxyConfig, instance_name: &str) -> String {
    if !ipc.auth_required || ipc.auth_user.is_empty() {
        return url.to_string();
    }
    let password = match super::keychain::get_instance_proxy_password(instance_name) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(target: "clawenv::proxy", "keychain password missing for instance={instance_name}: {e}");
            return url.to_string();
        }
    };
    // Split scheme://rest and inject user:pass@ at front of rest.
    for scheme in ["http://", "https://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            return format!("{scheme}{}:{}@{}", ipc.auth_user, password, rest);
        }
    }
    // Unknown scheme — default to http://
    format!("http://{}:{}@{}", ipc.auth_user, password, url)
}

/// Rewrite a URL's host part from `127.0.0.1` / `localhost` to
/// `new_host`. Non-loopback URLs pass through.
pub fn rewrite_loopback(url: &str, new_host: &str) -> String {
    if new_host == "127.0.0.1" {
        return url.to_string();
    }
    url.replace("127.0.0.1", new_host)
       .replace("://localhost", &format!("://{new_host}"))
}

// ---------------------------------------------------------------------------
// Apply helpers — push a resolved triple into various targets.
// ---------------------------------------------------------------------------

/// Inject the resolved triple into the current process's env so child
/// processes (reqwest, curl, npm, node) inherit it.
pub fn apply_env(triple: &ProxyTriple) {
    std::env::set_var("HTTP_PROXY", &triple.http);
    std::env::set_var("http_proxy", &triple.http);
    std::env::set_var("HTTPS_PROXY", &triple.https);
    std::env::set_var("https_proxy", &triple.https);
    std::env::set_var("NO_PROXY", &triple.no_proxy);
    std::env::set_var("no_proxy", &triple.no_proxy);
    tracing::debug!(target: "clawenv::proxy", "apply_env http={} source={:?}", triple.http, triple.source);
}

/// Clear proxy env vars — used when resolver returns `None` (direct).
pub fn clear_env() {
    for k in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy"] {
        std::env::remove_var(k);
    }
    tracing::debug!(target: "clawenv::proxy", "apply_env cleared (direct)");
}

/// Inject the resolved triple into a child `Command` being spawned,
/// without touching the parent's env. Used by Tauri IPC's install path
/// to ephemerally set proxy for one clawcli invocation.
pub fn apply_child_cmd(triple: &ProxyTriple, cmd: &mut tokio::process::Command) {
    cmd.env("HTTP_PROXY", &triple.http);
    cmd.env("http_proxy", &triple.http);
    cmd.env("HTTPS_PROXY", &triple.https);
    cmd.env("https_proxy", &triple.https);
    cmd.env("NO_PROXY", &triple.no_proxy);
    cmd.env("no_proxy", &triple.no_proxy);
}

/// Write `/etc/profile.d/proxy.sh` inside the sandbox VM and set npm
/// config. Always overwrites, so calling this with a fresh triple
/// replaces any previous values cleanly.
pub async fn apply_to_sandbox(
    triple: &ProxyTriple,
    backend: &dyn SandboxBackend,
) -> Result<()> {
    let esc_dq = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"export http_proxy="{}"
export https_proxy="{}"
export HTTP_PROXY="{}"
export HTTPS_PROXY="{}"
export no_proxy="{}"
export NO_PROXY="{}"
"#,
        esc_dq(&triple.http), esc_dq(&triple.https),
        esc_dq(&triple.http), esc_dq(&triple.https),
        esc_dq(&triple.no_proxy), esc_dq(&triple.no_proxy),
    );
    backend.exec(&format!(
        "cat > /etc/profile.d/proxy.sh << 'PROXYEOF'\n{script}PROXYEOF"
    )).await?;
    backend.exec("chmod +x /etc/profile.d/proxy.sh").await?;

    let esc_sq = |s: &str| s.replace('\'', "'\\''");
    backend.exec(&format!("npm config set proxy '{}'", esc_sq(&triple.http))).await.ok();
    backend.exec(&format!("npm config set https-proxy '{}'", esc_sq(&triple.https))).await.ok();

    tracing::info!(
        target: "clawenv::proxy",
        "apply_to_sandbox http={} source={:?}", triple.http, triple.source
    );
    Ok(())
}

/// Clear proxy inside sandbox — `mode == "none"` path.
pub async fn clear_sandbox(backend: &dyn SandboxBackend) -> Result<()> {
    backend.exec("rm -f /etc/profile.d/proxy.sh").await.ok();
    backend.exec("npm config delete proxy 2>/dev/null || true").await.ok();
    backend.exec("npm config delete https-proxy 2>/dev/null || true").await.ok();
    tracing::info!(target: "clawenv::proxy", "clear_sandbox done");
    Ok(())
}

fn trace_resolved(t: &ProxyTriple, scope_label: &str) {
    tracing::debug!(
        target: "clawenv::proxy",
        "resolve scope={scope_label} source={:?} http={}",
        t.source, t.http
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_loopback_lima() {
        assert_eq!(
            rewrite_loopback("http://127.0.0.1:7890", "host.lima.internal"),
            "http://host.lima.internal:7890"
        );
        assert_eq!(
            rewrite_loopback("http://localhost:7890", "host.lima.internal"),
            "http://host.lima.internal:7890"
        );
    }

    #[test]
    fn rewrite_loopback_native_noop() {
        assert_eq!(
            rewrite_loopback("http://127.0.0.1:7890", "127.0.0.1"),
            "http://127.0.0.1:7890"
        );
    }

    #[test]
    fn rewrite_loopback_non_local_passthrough() {
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
    fn read_env_triple_returns_none_when_clean() {
        // Save + clean env for the duration of the test. Serialize via a
        // local mutex — tests run in parallel and env is process-global.
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _g = LOCK.lock().unwrap();
        for k in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy"] {
            std::env::remove_var(k);
        }
        assert!(read_env_triple(ProxySource::ShellEnv).is_none());
    }

    #[test]
    fn triple_from_config_disabled_is_none() {
        let p = ProxyConfig {
            enabled: false,
            http_proxy: "http://1.2.3.4:8080".into(),
            https_proxy: "".into(),
            no_proxy: "localhost,127.0.0.1".into(),
            auth_required: false,
            auth_user: "".into(),
        };
        assert!(triple_from_config_proxy(&p, ProxySource::GlobalConfig).is_none());
    }

    #[test]
    fn triple_from_config_enabled_populates() {
        let p = ProxyConfig {
            enabled: true,
            http_proxy: "http://1.2.3.4:8080".into(),
            https_proxy: "".into(),
            no_proxy: "".into(),
            auth_required: false,
            auth_user: "".into(),
        };
        let t = triple_from_config_proxy(&p, ProxySource::GlobalConfig).unwrap();
        assert_eq!(t.http, "http://1.2.3.4:8080");
        assert_eq!(t.https, "http://1.2.3.4:8080"); // mirror of http
        assert_eq!(t.no_proxy, "localhost,127.0.0.1"); // default
    }
}
