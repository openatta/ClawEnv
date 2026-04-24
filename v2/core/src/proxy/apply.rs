//! Install a [`ProxyTriple`] inside a running sandbox. Mirrors v1's
//! `apply_to_sandbox`: writes `/etc/environment` for PAM-loaded
//! subprocesses and `/etc/profile.d/proxy.sh` for interactive shells,
//! plus `GIT_CONFIG_*` env vars that rewrite github SSH URLs to HTTPS
//! (so `npm install` postinstall scripts that do `git clone` don't
//! hit port-22 blocks).

use std::sync::Arc;

use crate::common::OpsError;
use crate::sandbox_backend::SandboxBackend;

use super::config::ProxyTriple;

/// Write the proxy triple into `/etc/environment` and
/// `/etc/profile.d/proxy.sh` inside the sandbox.
///
/// Uses the backend's `exec_argv` helper exclusively — every dynamic
/// value (URL, no_proxy list) is quoted, so no shell metacharacter in
/// a password can escape out and run arbitrary commands.
///
/// The two files are written with `sudo tee` (root-owned) followed by
/// `sudo chmod`. On busybox-based Alpine `tee` + `chmod` both come
/// from coreutils-minimal which is preinstalled.
pub async fn apply_to_sandbox(
    backend: &Arc<dyn SandboxBackend>,
    triple: &ProxyTriple,
) -> Result<(), OpsError> {
    let env_body = render_etc_environment(triple);
    let profile_body = render_profile_sh(triple);

    write_sandbox_file(backend, "/etc/environment", &env_body, "0644").await?;
    write_sandbox_file(backend, "/etc/profile.d/proxy.sh", &profile_body, "0755").await?;
    Ok(())
}

/// Remove the proxy files (graceful undo — missing files aren't an error).
pub async fn clear_sandbox_proxy(
    backend: &Arc<dyn SandboxBackend>,
) -> Result<(), OpsError> {
    for path in ["/etc/environment", "/etc/profile.d/proxy.sh"] {
        // Best-effort; `-f` makes missing files not an error.
        backend
            .exec_argv(&["sudo", "rm", "-f", path])
            .await
            .map_err(OpsError::Other)?;
    }
    Ok(())
}

// ——— Rendering (pure) ———

pub(crate) fn render_etc_environment(t: &ProxyTriple) -> String {
    let mut s = String::new();
    if !t.http.is_empty() {
        s.push_str(&format!("http_proxy={}\n", t.http));
        s.push_str(&format!("HTTP_PROXY={}\n", t.http));
    }
    if !t.https.is_empty() {
        s.push_str(&format!("https_proxy={}\n", t.https));
        s.push_str(&format!("HTTPS_PROXY={}\n", t.https));
    }
    if !t.no_proxy.is_empty() {
        s.push_str(&format!("no_proxy={}\n", t.no_proxy));
        s.push_str(&format!("NO_PROXY={}\n", t.no_proxy));
    }
    s.push_str("GIT_CONFIG_COUNT=1\n");
    s.push_str("GIT_CONFIG_KEY_0=url.https://github.com/.insteadOf\n");
    s.push_str("GIT_CONFIG_VALUE_0=ssh://git@github.com/\n");
    s
}

pub(crate) fn render_profile_sh(t: &ProxyTriple) -> String {
    let mut s = String::from("#!/bin/sh\n# managed by clawops proxy apply\n");
    let dq = esc_double_quote;
    if !t.http.is_empty() {
        s.push_str(&format!("export http_proxy=\"{}\"\n", dq(&t.http)));
        s.push_str(&format!("export HTTP_PROXY=\"{}\"\n", dq(&t.http)));
    }
    if !t.https.is_empty() {
        s.push_str(&format!("export https_proxy=\"{}\"\n", dq(&t.https)));
        s.push_str(&format!("export HTTPS_PROXY=\"{}\"\n", dq(&t.https)));
    }
    if !t.no_proxy.is_empty() {
        s.push_str(&format!("export no_proxy=\"{}\"\n", dq(&t.no_proxy)));
        s.push_str(&format!("export NO_PROXY=\"{}\"\n", dq(&t.no_proxy)));
    }
    s.push_str("export GIT_CONFIG_COUNT=1\n");
    s.push_str("export GIT_CONFIG_KEY_0=\"url.https://github.com/.insteadOf\"\n");
    s.push_str("export GIT_CONFIG_VALUE_0=\"ssh://git@github.com/\"\n");
    s
}

/// Double-quote escaping for POSIX shell. Mirrors v1's `esc_dq`.
fn esc_double_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '`' => out.push_str("\\`"),
            _ => out.push(c),
        }
    }
    out
}

// ——— I/O ———

/// Push `body` to `path` inside the sandbox via `sudo tee`. Uses a
/// heredoc with a randomised marker so nothing in `body` can
/// prematurely terminate it.
async fn write_sandbox_file(
    backend: &Arc<dyn SandboxBackend>,
    path: &str,
    body: &str,
    mode: &str,
) -> Result<(), OpsError> {
    // Marker must not appear in body. We pick a long random hex which
    // is astronomically unlikely to collide with real content.
    let marker = format!("CLAWOPS_EOF_{}", random_hex(16));
    // Build the command with structured quoting via exec_argv.
    // We can't pipe through exec_argv directly; use sh -c and let the
    // *heredoc body* contain the user text — metachars are inert inside
    // a quoted heredoc (<<'EOF'), no escaping needed.
    let script = format!(
        "cat <<'{marker}' | sudo tee {path} >/dev/null && sudo chmod {mode} {path}\n{body}\n{marker}\n"
    );
    backend
        .exec_argv(&["sh", "-c", &script])
        .await
        .map_err(OpsError::Other)?;
    Ok(())
}

fn random_hex(n_bytes: usize) -> String {
    // Cheap, non-crypto randomness — this is an anti-collision marker,
    // not a secret. Avoid pulling in `rand` just for this.
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ptr = &nanos as *const _ as usize;
    let seed = nanos ^ (ptr as u128);
    let mut out = String::with_capacity(n_bytes * 2);
    for i in 0..n_bytes {
        let byte = ((seed >> (i * 8)) & 0xff) as u8;
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::config::ProxySource;

    fn sample_triple() -> ProxyTriple {
        ProxyTriple {
            http: "http://alice:s3cret@proxy:3128".into(),
            https: "http://alice:s3cret@proxy:3128".into(),
            no_proxy: "localhost,127.0.0.1,github.com".into(),
            source: ProxySource::GlobalConfig,
        }
    }

    #[test]
    fn env_file_has_both_cases_and_git_rewrite() {
        let s = render_etc_environment(&sample_triple());
        assert!(s.contains("http_proxy=http://alice:s3cret@proxy:3128"));
        assert!(s.contains("HTTP_PROXY=http://alice:s3cret@proxy:3128"));
        assert!(s.contains("no_proxy=localhost,127.0.0.1,github.com"));
        assert!(s.contains("GIT_CONFIG_COUNT=1"));
        assert!(s.contains("GIT_CONFIG_KEY_0=url.https://github.com/.insteadOf"));
        assert!(s.contains("GIT_CONFIG_VALUE_0=ssh://git@github.com/"));
    }

    #[test]
    fn profile_sh_uses_export_and_escapes() {
        let s = render_profile_sh(&sample_triple());
        assert!(s.starts_with("#!/bin/sh"));
        assert!(s.contains("export http_proxy=\""));
        assert!(s.contains("export GIT_CONFIG_KEY_0=\"url.https://github.com/.insteadOf\""));
    }

    #[test]
    fn profile_sh_escapes_backticks_and_dollars() {
        let triple = ProxyTriple {
            http: "http://evil`rm -rf /`@p:3128".into(),
            https: String::new(),
            no_proxy: "$(whoami)".into(),
            source: ProxySource::GlobalConfig,
        };
        let s = render_profile_sh(&triple);
        // The backtick must have been escaped.
        assert!(s.contains("\\`rm -rf /\\`"), "backtick not escaped: {s}");
        // $( must have been escaped too.
        assert!(s.contains("\\$(whoami)"), "dollar-paren not escaped: {s}");
    }

    #[test]
    fn env_file_skips_empty_urls() {
        let t = ProxyTriple {
            http: String::new(),
            https: String::new(),
            no_proxy: "localhost".into(),
            source: ProxySource::GlobalConfig,
        };
        let s = render_etc_environment(&t);
        assert!(!s.contains("http_proxy="));
        assert!(s.contains("no_proxy=localhost"));
        // Git rewrite is always present, even without proxy, because
        // it's useful whenever we've decided to touch the sandbox.
        assert!(s.contains("GIT_CONFIG_COUNT=1"));
    }

    #[test]
    fn esc_dq_handles_all_special_chars() {
        assert_eq!(esc_double_quote(r#"a"b"#), r#"a\"b"#);
        assert_eq!(esc_double_quote(r"a\b"), r"a\\b");
        assert_eq!(esc_double_quote("a$b"), r"a\$b");
        assert_eq!(esc_double_quote("a`b"), r"a\`b");
    }

    #[test]
    fn random_hex_produces_expected_length() {
        assert_eq!(random_hex(16).len(), 32);
        assert_eq!(random_hex(4).len(), 8);
    }

    #[tokio::test]
    async fn apply_to_sandbox_invokes_backend_with_both_files() {
        use crate::sandbox_ops::testing::MockBackend;
        let mock = Arc::new(MockBackend::new("fake"));
        let b: Arc<dyn SandboxBackend> = mock.clone();
        apply_to_sandbox(&b, &sample_triple()).await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        assert_eq!(log.len(), 2, "expected 2 exec calls (env + profile)");
        assert!(log[0].contains("/etc/environment"));
        assert!(log[1].contains("/etc/profile.d/proxy.sh"));
        // Heredoc body should appear in the exec'd shell script.
        assert!(log[0].contains("HTTP_PROXY="));
    }

    #[tokio::test]
    async fn clear_sandbox_proxy_removes_both_files() {
        use crate::sandbox_ops::testing::MockBackend;
        let mock = Arc::new(MockBackend::new("fake"));
        let b: Arc<dyn SandboxBackend> = mock.clone();
        clear_sandbox_proxy(&b).await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        assert_eq!(log.len(), 2);
        assert!(log[0].contains("/etc/environment"));
        assert!(log[1].contains("/etc/profile.d/proxy.sh"));
        // Uses rm -f, not cat/tee.
        assert!(log[0].contains("rm"));
    }
}
