//! Apply mirror configuration inside a running sandbox. Writes
//! `/etc/apk/repositories` and `npm config set registry`.
//!
//! Ported from v1's `config/mirrors.rs`. Differences:
//!
//! - v1 loaded defaults from `assets/mirrors.toml` via a runtime
//!   include_str!. v2 hardcodes the two URLs as constants — there are
//!   exactly two (alpine + npm) and they're upstream-only after v0.3.0,
//!   so a TOML indirection isn't buying anything.
//! - v1's `backend.exec(&str)` took raw shell; v2 routes through
//!   `exec_argv` so the mirror URL can't smuggle metacharacters.
//! - The heredoc pattern mirrors what `proxy::apply::apply_to_sandbox`
//!   does: `sh -c` with `<<'MARKER'` keeps URL content inert.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::common::OpsError;
use crate::sandbox_backend::SandboxBackend;

/// Upstream Alpine CDN. Matches v1 `assets/mirrors.toml [apk] official_base_urls`.
pub const DEFAULT_ALPINE_REPO: &str = "https://dl-cdn.alpinelinux.org/alpine";

/// Upstream npm registry. Matches v1 `assets/mirrors.toml [npm] official_urls`.
pub const DEFAULT_NPM_REGISTRY: &str = "https://registry.npmjs.org";

/// Alpine version to fall back on when detection fails inside the VM.
/// Matches what v1 uses (`config/mirrors.rs:29`).
const FALLBACK_ALPINE_MAJOR_MINOR: &str = "3.23";

/// User-settable mirror overrides. Empty string = use upstream default.
///
/// Serde shape aligned with v1's `[clawenv.mirrors]` so a config.toml
/// written by v1 deserialises cleanly here (R3-D4 follow-up).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MirrorsConfig {
    pub alpine_repo: String,
    pub npm_registry: String,
}

impl MirrorsConfig {
    pub fn alpine_repo_url(&self) -> &str {
        if self.alpine_repo.is_empty() {
            DEFAULT_ALPINE_REPO
        } else {
            self.alpine_repo.as_str()
        }
    }

    pub fn npm_registry_url(&self) -> &str {
        if self.npm_registry.is_empty() {
            DEFAULT_NPM_REGISTRY
        } else {
            self.npm_registry.as_str()
        }
    }

    /// True when both fields are empty (pure defaults).
    pub fn is_default(&self) -> bool {
        self.alpine_repo.is_empty() && self.npm_registry.is_empty()
    }
}

/// Apply mirror configuration inside a live sandbox.
///
/// 1. Detect Alpine major.minor from `/etc/alpine-release` (fallback: 3.23).
/// 2. Write `/etc/apk/repositories` = two lines (`<base>/v<x.y>/main`,
///    `<base>/v<x.y>/community`) via `sudo tee`.
/// 3. If user overrode npm registry, run `npm config set registry <url>`
///    (non-sudo — writes to `~/.npmrc`). Default upstream is a no-op
///    because npm already ships with it.
pub async fn apply_mirrors(
    backend: &Arc<dyn SandboxBackend>,
    mirrors: &MirrorsConfig,
) -> Result<(), OpsError> {
    let alpine_version = detect_alpine_major_minor(backend).await;
    let repos_body = format_apk_repositories(mirrors.alpine_repo_url(), &alpine_version);

    // Write /etc/apk/repositories with `sudo tee`. Heredoc marker is
    // inert — URL content can't smuggle shell metacharacters. Uses
    // exec_argv_with_retry: this path runs right after VM boot when
    // Lima's SSH ControlMaster is racy (v1 v0.2.10 lesson).
    let marker = "CLAWOPS_APK_EOF";
    let script = format!(
        "sudo tee /etc/apk/repositories >/dev/null << '{marker}'\n{repos_body}{marker}\n"
    );
    backend
        .exec_argv_with_retry(&["sh", "-c", &script])
        .await
        .map_err(OpsError::Other)?;

    let npm = mirrors.npm_registry_url();
    if npm != DEFAULT_NPM_REGISTRY {
        // Non-sudo — writes to the user's own ~/.npmrc.
        backend
            .exec_argv_with_retry(&["npm", "config", "set", "registry", npm])
            .await
            .map_err(OpsError::Other)?;
    }
    Ok(())
}

/// Probe `/etc/alpine-release` inside the VM and return the "major.minor"
/// token (e.g. `3.23`). Graceful fallback on any failure — Alpine
/// versions rarely change, so `3.23` stays correct on most VMs for the
/// full support window of a given mirrors.toml release.
async fn detect_alpine_major_minor(backend: &Arc<dyn SandboxBackend>) -> String {
    let out = match backend.exec_argv(&["cat", "/etc/alpine-release"]).await {
        Ok(s) => s,
        Err(_) => return FALLBACK_ALPINE_MAJOR_MINOR.into(),
    };
    parse_major_minor(&out).unwrap_or_else(|| FALLBACK_ALPINE_MAJOR_MINOR.into())
}

/// Parse "3.23.2\n" → "3.23". Returns None on empty or malformed input.
fn parse_major_minor(s: &str) -> Option<String> {
    let line = s.trim();
    if line.is_empty() {
        return None;
    }
    let mut parts = line.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    // Sanity: both should be numeric.
    if major.parse::<u32>().is_err() || minor.parse::<u32>().is_err() {
        return None;
    }
    Some(format!("{major}.{minor}"))
}

/// Render the two-line `/etc/apk/repositories` body.
///
/// Pure function — no I/O. Factored out for unit testing.
pub fn format_apk_repositories(base: &str, major_minor: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/v{major_minor}/main\n{base}/v{major_minor}/community\n")
}

/// Generate the shell snippet that applies apk + npm mirrors inline
/// during cloud-init (first-boot, before the VM is accessible via
/// exec_argv). Used by Lima YAML template and Podman Containerfile.
///
/// Emits:
/// - Runtime detection of Alpine major.minor
/// - Overwrite `/etc/apk/repositories` with the two-line repo body
/// - Optional `npm config set registry` when user overrode default
pub fn provision_snippet(mirrors: &MirrorsConfig) -> String {
    let base = mirrors.alpine_repo_url().trim_end_matches('/');
    let npm = mirrors.npm_registry_url();
    let mut s = String::new();
    s.push_str(
        "ALPINE_VER=$(cat /etc/alpine-release 2>/dev/null | cut -d. -f1,2)\n\
         if [ -z \"$ALPINE_VER\" ]; then ALPINE_VER='",
    );
    s.push_str(FALLBACK_ALPINE_MAJOR_MINOR);
    s.push_str(
        "'; fi\n\
         printf '%s/v%s/main\\n%s/v%s/community\\n' '",
    );
    s.push_str(base);
    s.push_str("' \"$ALPINE_VER\" '");
    s.push_str(base);
    s.push_str("' \"$ALPINE_VER\" > /etc/apk/repositories\n");
    if npm != DEFAULT_NPM_REGISTRY {
        s.push_str("npm config set registry '");
        s.push_str(npm);
        s.push_str("' 2>/dev/null || true\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_ops::testing::MockBackend;

    /// Returns (concrete Arc for reading exec_log, trait-object Arc for
    /// apply_mirrors). Pattern mirrors proxy::apply tests.
    fn arc_mock(stdout: &str) -> (Arc<MockBackend>, Arc<dyn SandboxBackend>) {
        let concrete = Arc::new(MockBackend::new("fake").with_stdout(stdout));
        let as_trait: Arc<dyn SandboxBackend> = concrete.clone();
        (concrete, as_trait)
    }

    // ——— parse_major_minor ———

    #[test]
    fn parse_major_minor_happy() {
        assert_eq!(parse_major_minor("3.23.2\n"), Some("3.23".into()));
        assert_eq!(parse_major_minor("3.23.2"), Some("3.23".into()));
        assert_eq!(parse_major_minor("  3.20.1\n"), Some("3.20".into()));
    }

    #[test]
    fn parse_major_minor_major_only_is_none() {
        assert_eq!(parse_major_minor("3"), None);
    }

    #[test]
    fn parse_major_minor_non_numeric_is_none() {
        assert_eq!(parse_major_minor("v3.23.2"), None);
        assert_eq!(parse_major_minor("edge"), None);
    }

    #[test]
    fn parse_major_minor_empty_is_none() {
        assert_eq!(parse_major_minor(""), None);
        assert_eq!(parse_major_minor("\n"), None);
    }

    // ——— format_apk_repositories ———

    #[test]
    fn format_uses_upstream_default() {
        let body = format_apk_repositories(DEFAULT_ALPINE_REPO, "3.23");
        assert_eq!(
            body,
            "https://dl-cdn.alpinelinux.org/alpine/v3.23/main\n\
             https://dl-cdn.alpinelinux.org/alpine/v3.23/community\n"
        );
    }

    #[test]
    fn format_trims_trailing_slash() {
        let body = format_apk_repositories("https://example.com/alpine/", "3.23");
        // No double slash despite input.
        assert!(!body.contains("//v3.23"));
        assert!(body.contains("https://example.com/alpine/v3.23/main"));
    }

    #[test]
    fn format_two_lines() {
        let body = format_apk_repositories(DEFAULT_ALPINE_REPO, "3.22");
        assert_eq!(body.lines().count(), 2);
        assert!(body.lines().next().unwrap().ends_with("/main"));
    }

    // ——— MirrorsConfig ———

    #[test]
    fn mirrors_defaults_match_upstream() {
        let m = MirrorsConfig::default();
        assert_eq!(m.alpine_repo_url(), DEFAULT_ALPINE_REPO);
        assert_eq!(m.npm_registry_url(), DEFAULT_NPM_REGISTRY);
        assert!(m.is_default());
    }

    #[test]
    fn mirrors_user_override_wins() {
        let m = MirrorsConfig {
            alpine_repo: "https://mirrors.example.com/alpine".into(),
            npm_registry: "https://npm.example.com".into(),
        };
        assert_eq!(m.alpine_repo_url(), "https://mirrors.example.com/alpine");
        assert_eq!(m.npm_registry_url(), "https://npm.example.com");
        assert!(!m.is_default());
    }

    #[test]
    fn mirrors_partial_override() {
        let m = MirrorsConfig {
            alpine_repo: "https://a.example.com/alpine".into(),
            ..Default::default()
        };
        assert_eq!(m.alpine_repo_url(), "https://a.example.com/alpine");
        assert_eq!(m.npm_registry_url(), DEFAULT_NPM_REGISTRY);
    }

    #[test]
    fn mirrors_roundtrips_toml() {
        let m = MirrorsConfig {
            alpine_repo: "https://a".into(),
            npm_registry: "https://n".into(),
        };
        let t = toml::to_string(&m).unwrap();
        let back: MirrorsConfig = toml::from_str(&t).unwrap();
        assert_eq!(back, m);
    }

    // ——— apply_mirrors via MockBackend ———

    #[tokio::test]
    async fn apply_defaults_writes_repos_and_skips_npm() {
        let (mock, backend) = arc_mock("3.23.2\n");
        apply_mirrors(&backend, &MirrorsConfig::default()).await.unwrap();
        let log = mock.exec_log.lock().unwrap().clone();
        assert_eq!(log.len(), 2, "expected 2 exec calls (detect + write), got {log:?}");
        assert!(log[0].contains("cat"));
        assert!(log[0].contains("/etc/alpine-release"));
        assert!(log[1].contains("tee"));
        assert!(log[1].contains("/etc/apk/repositories"));
        assert!(log[1].contains("dl-cdn.alpinelinux.org"));
        assert!(log[1].contains("/v3.23/main"));
    }

    #[tokio::test]
    async fn apply_with_npm_override_runs_config_set() {
        let (mock, backend) = arc_mock("3.23.2\n");
        let m = MirrorsConfig {
            alpine_repo: String::new(),
            npm_registry: "https://npm.example.com".into(),
        };
        apply_mirrors(&backend, &m).await.unwrap();
        let log = mock.exec_log.lock().unwrap().clone();
        assert_eq!(log.len(), 3, "expected detect + tee + npm config set, got {log:?}");
        assert!(log[2].contains("npm"));
        assert!(log[2].contains("config"));
        assert!(log[2].contains("https://npm.example.com"));
    }

    #[tokio::test]
    async fn apply_falls_back_when_version_detect_fails() {
        // Empty stdout → parse_major_minor returns None → fallback kicks in.
        let (mock, backend) = arc_mock("");
        apply_mirrors(&backend, &MirrorsConfig::default()).await.unwrap();
        let log = mock.exec_log.lock().unwrap().clone();
        assert!(log[1].contains("/v3.23/main"), "fallback 3.23 missing: {:?}", log[1]);
    }

    #[tokio::test]
    async fn apply_with_alpine_repo_override_uses_it() {
        let (mock, backend) = arc_mock("3.23.2");
        let m = MirrorsConfig {
            alpine_repo: "https://mirrors.example.com/alpine".into(),
            npm_registry: String::new(),
        };
        apply_mirrors(&backend, &m).await.unwrap();
        let log = mock.exec_log.lock().unwrap().clone();
        assert!(log[1].contains("https://mirrors.example.com/alpine"));
        assert!(!log[1].contains("dl-cdn.alpinelinux.org"));
    }

    // ——— provision_snippet ———

    #[test]
    fn provision_snippet_contains_fallback_version() {
        let s = provision_snippet(&MirrorsConfig::default());
        assert!(s.contains("ALPINE_VER="));
        assert!(s.contains(FALLBACK_ALPINE_MAJOR_MINOR));
    }

    #[test]
    fn provision_snippet_skips_npm_when_default() {
        let s = provision_snippet(&MirrorsConfig::default());
        assert!(!s.contains("npm config set"));
    }

    #[test]
    fn provision_snippet_includes_npm_when_overridden() {
        let m = MirrorsConfig {
            alpine_repo: String::new(),
            npm_registry: "https://npm.example.com".into(),
        };
        let s = provision_snippet(&m);
        assert!(s.contains("npm config set registry"));
        assert!(s.contains("https://npm.example.com"));
    }

    #[test]
    fn provision_snippet_uses_custom_alpine_base() {
        let m = MirrorsConfig {
            alpine_repo: "https://mirrors.example.com/alpine/".into(),
            npm_registry: String::new(),
        };
        let s = provision_snippet(&m);
        assert!(s.contains("https://mirrors.example.com/alpine"));
        // Trailing slash trimmed — shouldn't see //v$ALPINE_VER.
        assert!(!s.contains("//v"));
    }

}
