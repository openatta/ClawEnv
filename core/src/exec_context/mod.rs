//! ExecutionContext — unified surface for "run a command somewhere".
//!
//! v1's pattern was: business logic branched on "is this sandbox or
//! native?" at every step (`if inst.sandbox_type == Native { ... }
//! else { backend.exec(...) }`). v2's design (CLI-DESIGN.md §6)
//! collapses that into a single trait. Verbs that don't care WHERE
//! they run accept `Arc<dyn ExecutionContext>` and the install
//! pipeline / token reader / log tail / doctor all reuse one
//! implementation.
//!
//! Two implementations:
//! - `SandboxContext` — wraps a [`SandboxBackend`]; exec routes
//!   through `limactl shell` / `wsl --exec` / `podman exec`.
//! - `NativeContext` — wraps `tokio::process::Command` rooted at the
//!   instance's native install prefix.
//!
//! `MockContext` lives in the `testing` submodule for unit-test reuse.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::sandbox_backend::SandboxBackend;
use crate::sandbox_ops::BackendKind;

pub mod native;
pub mod sandbox;
pub mod testing;

pub use native::NativeContext;
pub use sandbox::SandboxContext;

/// Where this context maps in the abstract space. Returned by
/// [`ExecutionContext::kind`] so verb implementations can branch when
/// they have to (e.g. "version probe is exec'd inside VM but read
/// from a file on native").
#[derive(Debug, Clone)]
pub enum ContextKind {
    Sandbox { backend: BackendKind, instance: String },
    Native { prefix: PathBuf },
}

/// Structured exec error. Mirrors the v1 transient-vs-fatal split so
/// retry logic can decide cleanly.
#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("transient: {0}")]
    Transient(String),
    #[error("non-zero exit ({code}): {stderr_tail}")]
    NonZero { code: i32, stderr_tail: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// The unified execution surface. Used everywhere business logic
/// needs to run a command "somewhere" without caring whether the
/// somewhere is a Lima VM or the host process tree.
#[async_trait]
pub trait ExecutionContext: Send + Sync {
    /// Stable identifier (e.g. "lima:default", "podman:abc123",
    /// "native:/Users/x/.clawenv/native/default"). Mostly for logs
    /// and error messages.
    fn id(&self) -> String;

    /// Where this context lands in the abstract space.
    fn kind(&self) -> ContextKind;

    /// One-shot exec. Returns stdout on success.
    async fn exec(&self, argv: &[&str]) -> Result<String, ExecError>;

    /// Same with retry on transient errors. Default impl: 5 attempts
    /// with backoff [0, 1s, 3s, 9s, 30s], retries only on
    /// `ExecError::Transient`. Backends with a tighter retry budget
    /// (or none) override.
    async fn exec_with_retry(&self, argv: &[&str]) -> Result<String, ExecError> {
        let delays_ms: [u64; 5] = [0, 1_000, 3_000, 9_000, 30_000];
        let mut last_err: Option<ExecError> = None;
        for (i, &d) in delays_ms.iter().enumerate() {
            if d > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(d)).await;
                tracing::debug!(
                    target: "clawenv::exec",
                    "[{}] retry attempt {} after {}ms backoff",
                    self.id(), i + 1, d
                );
            }
            match self.exec(argv).await {
                Ok(s) => return Ok(s),
                Err(e @ ExecError::Transient(_)) => {
                    tracing::warn!(
                        target: "clawenv::exec",
                        "[{}] attempt {} transient: {e}", self.id(), i + 1
                    );
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| ExecError::Other("retries exhausted".into())))
    }

    /// Whether this context is currently usable (VM running, host has
    /// the binary, etc.). Used by `clawcli doctor` and pre-flight
    /// gates. Default: try a no-op exec (`true` command) and observe.
    async fn is_alive(&self) -> bool {
        self.exec(&["true"]).await.is_ok()
    }

    /// Resolve an in-context path to a host path if a host mount or
    /// shared filesystem covers it. Native: passthrough. Sandbox:
    /// returns Some only when the path falls under a known mount
    /// (workspace, /tmp/clawenv-shared, etc.) — None otherwise.
    fn resolve_to_host(&self, ctx_path: &Path) -> Option<PathBuf>;

    /// Long-running streaming exec — drives `on_line` for every line
    /// of stdout. Used for install pipelines (apk update / npm
    /// install) where progress feedback matters. Returns the child's
    /// exit code on completion.
    ///
    /// Default impl: blocking `exec` with no streaming. Backends that
    /// can stream (sandbox via PTY, native via tokio process pipe)
    /// override.
    async fn exec_streaming(
        &self,
        argv: &[&str],
        on_line: &mut (dyn FnMut(String) + Send),
    ) -> Result<i32, ExecError> {
        // Block on exec; send stdout in one chunk to satisfy the
        // streaming contract minimally. Better than ignoring on_line.
        let stdout = self.exec(argv).await?;
        for line in stdout.lines() {
            on_line(line.to_string());
        }
        Ok(0)
    }
}

// Sandbox backends often surface SSH master races / connection
// resets. The transient pattern set is shared between
// `SandboxContext` and the older `SandboxBackend::is_transient_ssh_error`
// helper — keep one source of truth here.
pub(crate) fn is_transient_exec_error(msg: &str) -> bool {
    msg.contains("exit 255")
        || msg.contains("Connection reset")
        || msg.contains("kex_exchange_identification")
        || msg.contains("Connection refused")
        || msg.contains("Broken pipe")
        || msg.contains("ssh: connect")
}

/// Convenience: build the right ExecutionContext for an instance based
/// on its backend kind. Returns `None` only when `backend` is a sandbox
/// kind whose impl isn't compiled in (won't happen in practice).
pub fn for_instance(
    backend: BackendKind,
    instance_name: &str,
    _native_prefix: Option<PathBuf>,
) -> Option<Arc<dyn ExecutionContext>> {
    use crate::sandbox_backend::{LimaBackend, PodmanBackend, WslBackend};
    match backend {
        BackendKind::Lima => {
            let b: Arc<dyn SandboxBackend> = Arc::new(LimaBackend::new(instance_name));
            Some(Arc::new(SandboxContext::new(b, BackendKind::Lima, instance_name.into())))
        }
        BackendKind::Wsl2 => {
            let b: Arc<dyn SandboxBackend> = Arc::new(WslBackend::new(instance_name));
            Some(Arc::new(SandboxContext::new(b, BackendKind::Wsl2, instance_name.into())))
        }
        BackendKind::Podman => {
            let b: Arc<dyn SandboxBackend> = Arc::new(PodmanBackend::new(instance_name));
            Some(Arc::new(SandboxContext::new(b, BackendKind::Podman, instance_name.into())))
        }
    }
}

/// Native variant — separate fn because BackendKind enum doesn't have
/// a Native variant (it's a sandbox-kinds-only enum). Caller picks
/// based on the instance's SandboxKind.
pub fn for_native(prefix: PathBuf) -> Arc<dyn ExecutionContext> {
    Arc::new(NativeContext::new(prefix))
}
