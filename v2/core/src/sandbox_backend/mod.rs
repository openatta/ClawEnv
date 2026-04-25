//! v2-native SandboxBackend trait + implementations.
//!
//! Covers the full VM lifecycle: create → start → exec/stats/port → stop →
//! destroy. `create()` consumes a [`CreateOpts`] assembled by
//! `provisioning::templates` and is what makes v2 self-sufficient
//! (no longer delegating VM bootstrap to v1).

pub mod lima;
pub mod wsl;
pub mod podman;

use async_trait::async_trait;

pub use lima::LimaBackend;
pub use podman::PodmanBackend;
pub use wsl::WslBackend;

use crate::provisioning::CreateOpts;

/// Capabilities and runtime state of a sandbox backend. Mirrors v2's
/// `SandboxOps` needs.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Human-readable name (e.g. "Lima", "WSL2", "Podman").
    fn name(&self) -> &str;

    /// Instance identifier (e.g. "default").
    fn instance(&self) -> &str;

    /// Is the underlying tool + VM available / running right now?
    async fn is_available(&self) -> anyhow::Result<bool>;

    /// Does the VM / container definition exist on this host, regardless
    /// of run state? A true result means we should NOT call `create()`
    /// (it would clash or double-provision); we should just `start()`.
    ///
    /// Default impl is `is_available()` — backends that distinguish
    /// "defined but stopped" from "running" should override.
    async fn is_present(&self) -> anyhow::Result<bool> {
        self.is_available().await
    }

    /// Provision a fresh VM / container. Renders the backend's template
    /// (Lima YAML, WSL rootfs import + provision script, Podman build),
    /// invokes the backend tool, blocks until provision completes.
    ///
    /// Idempotency: backends SHOULD check `is_present()` first; if true,
    /// return `Ok(())` without reprovisioning so callers can invoke
    /// `create()` as a no-op readiness guarantee.
    async fn create(&self, opts: &CreateOpts) -> anyhow::Result<()>;

    /// Tear down the VM / container and remove its backing files.
    /// Must be idempotent — destroying a missing instance is a no-op.
    async fn destroy(&self) -> anyhow::Result<()>;

    /// Start the VM (idempotent — noop if already running).
    async fn start(&self) -> anyhow::Result<()>;

    /// Stop the VM (idempotent — noop if already stopped).
    async fn stop(&self) -> anyhow::Result<()>;

    /// Run a **raw shell command** inside the VM and return stdout on success.
    /// Error includes stderr snippet on non-zero exit.
    ///
    /// # Safety
    /// `cmd` is passed to an in-VM `sh -c` verbatim. Callers MUST have
    /// composed it from trusted, constant sources, or pre-quoted every
    /// dynamic fragment. For any case where you have structured arguments
    /// (binary + args), prefer [`exec_argv`](Self::exec_argv) — it quotes
    /// each piece for you.
    async fn exec(&self, cmd: &str) -> anyhow::Result<String>;

    /// Run a command inside the VM with structured argv. Each element is
    /// POSIX-quoted before composition into a shell command, so this path
    /// is safe for arbitrary argument content (paths with spaces, values
    /// from config, parsed IP addresses, etc.).
    ///
    /// Default impl quotes args and delegates to [`exec`](Self::exec). A
    /// backend that can invoke without `sh -c` wrapping (e.g. direct
    /// process spawn into the VM namespace) may override this.
    async fn exec_argv(&self, argv: &[&str]) -> anyhow::Result<String> {
        if argv.is_empty() {
            anyhow::bail!("exec_argv: empty argv");
        }
        let quoted = argv.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
        self.exec(&quoted).await
    }

    /// Run a command with retry on transient SSH/networking errors that
    /// commonly happen right after VM boot (Lima's SSH ControlMaster
    /// hasn't warmed up yet, kex_exchange_identification reset, etc.).
    ///
    /// Backoff schedule: 0ms → 1s → 3s → 9s (4 attempts total). Mirrors
    /// v1 `proxy_resolver::exec_with_retry`.
    ///
    /// Default impl wraps [`exec_argv`](Self::exec_argv); backends that
    /// know their own transient error patterns can override.
    async fn exec_argv_with_retry(&self, argv: &[&str]) -> anyhow::Result<String> {
        let delays_ms: [u64; 4] = [0, 1_000, 3_000, 9_000];
        let mut last_err: Option<anyhow::Error> = None;
        for (i, &d) in delays_ms.iter().enumerate() {
            if d > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(d)).await;
                tracing::debug!(
                    target: "clawenv::backend",
                    "exec_argv_with_retry attempt {} after {}ms backoff", i + 1, d
                );
            }
            match self.exec_argv(argv).await {
                Ok(out) => return Ok(out),
                Err(e) => {
                    let msg = format!("{e}");
                    if !is_transient_ssh_error(&msg) {
                        return Err(e);
                    }
                    tracing::warn!(
                        target: "clawenv::backend",
                        "exec_argv_with_retry attempt {} transient: {msg}", i + 1
                    );
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("exec_argv_with_retry: retries exhausted")))
    }

    /// Current resource usage.
    async fn stats(&self) -> anyhow::Result<ResourceStats>;

    /// Replace the full port-forward table with this set.
    /// Individual add/remove is modeled by reading current state + rewriting
    /// at the `SandboxOps` layer.
    async fn edit_port_forwards(&self, forwards: &[(u16, u16)]) -> anyhow::Result<()>;

    fn supports_rename(&self) -> bool { false }
    fn supports_resource_edit(&self) -> bool { false }
    fn supports_port_edit(&self) -> bool { false }
}

/// Returns true when an exec error string matches a known transient
/// SSH/networking failure that's worth retrying. Mirrors v1
/// `proxy_resolver::exec_with_retry`'s pattern set, learned from
/// real Lima boot races (CHANGELOG v0.2.10).
pub(crate) fn is_transient_ssh_error(msg: &str) -> bool {
    msg.contains("exit 255")
        || msg.contains("Connection reset")
        || msg.contains("kex_exchange_identification")
        || msg.contains("Connection refused")
        || msg.contains("Broken pipe")
        || msg.contains("ssh: connect")
}

/// POSIX shell single-quoting. Mirrors v1's `platform::shell_quote`. Safe
/// for every byte including single quotes: `o'brien` → `'o'\''brien'`.
pub(crate) fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[derive(Debug, Clone, Default)]
pub struct ResourceStats {
    pub cpu_percent: f32,
    pub memory_used_mb: u64,
    pub memory_limit_mb: u64,
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn shell_quote_simple_word() {
        assert_eq!(shell_quote("nslookup"), "'nslookup'");
    }

    #[test]
    fn shell_quote_with_space() {
        assert_eq!(shell_quote("hello world"), "'hello world'");
    }

    #[test]
    fn shell_quote_with_single_quote() {
        assert_eq!(shell_quote("o'brien"), r"'o'\''brien'");
    }

    #[test]
    fn shell_quote_metacharacters_are_inert() {
        assert_eq!(
            shell_quote("$(rm -rf /); echo pwn"),
            "'$(rm -rf /); echo pwn'"
        );
    }

    #[test]
    fn shell_quote_empty_becomes_empty_string_literal() {
        assert_eq!(shell_quote(""), "''");
    }

    // ——— is_transient_ssh_error ———

    #[test]
    fn transient_matches_kex_reset() {
        assert!(super::is_transient_ssh_error(
            "exec failed (exit 255): kex_exchange_identification: read: Connection reset by peer"
        ));
    }

    #[test]
    fn transient_matches_broken_pipe() {
        assert!(super::is_transient_ssh_error(
            "mux_client_request_session: read from master failed: Broken pipe"
        ));
    }

    #[test]
    fn transient_matches_connection_refused() {
        assert!(super::is_transient_ssh_error("ssh: connect: Connection refused"));
    }

    #[test]
    fn transient_does_not_match_real_command_failures() {
        // A genuine command exit-2 (e.g. file not found) should NOT
        // be retried — that's a permanent, useful error.
        assert!(!super::is_transient_ssh_error(
            "command failed (exit 2): ls: /no/such/path: No such file"
        ));
        assert!(!super::is_transient_ssh_error("Permission denied"));
    }
}
