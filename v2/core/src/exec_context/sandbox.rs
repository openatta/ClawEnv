//! `ExecutionContext` impl that runs commands inside a sandbox VM/
//! container by delegating to `SandboxBackend::exec_argv`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::sandbox_backend::SandboxBackend;
use crate::sandbox_ops::BackendKind;

use super::{is_transient_exec_error, ContextKind, ExecError, ExecutionContext};

pub struct SandboxContext {
    backend: Arc<dyn SandboxBackend>,
    kind: BackendKind,
    instance: String,
}

impl SandboxContext {
    pub fn new(backend: Arc<dyn SandboxBackend>, kind: BackendKind, instance: String) -> Self {
        Self { backend, kind, instance }
    }
}

#[async_trait]
impl ExecutionContext for SandboxContext {
    fn id(&self) -> String {
        format!("{}:{}", format!("{:?}", self.kind).to_lowercase(), self.instance)
    }

    fn kind(&self) -> ContextKind {
        ContextKind::Sandbox {
            backend: self.kind,
            instance: self.instance.clone(),
        }
    }

    async fn exec(&self, argv: &[&str]) -> Result<String, ExecError> {
        // Map anyhow → structured ExecError so retry logic can branch.
        match self.backend.exec_argv(argv).await {
            Ok(s) => Ok(s),
            Err(e) => {
                let msg = format!("{e}");
                if is_transient_exec_error(&msg) {
                    Err(ExecError::Transient(msg))
                } else {
                    // Best-effort: parse exit code out of the v1 message
                    // shape "<tool> exec failed (exit N): stderr: ...".
                    let code = msg.split("exit ").nth(1)
                        .and_then(|t| t.split(')').next())
                        .and_then(|n| n.trim().parse().ok())
                        .unwrap_or(1);
                    Err(ExecError::NonZero { code, stderr_tail: msg })
                }
            }
        }
    }

    async fn is_alive(&self) -> bool {
        self.backend.is_available().await.unwrap_or(false)
    }

    fn resolve_to_host(&self, ctx_path: &Path) -> Option<PathBuf> {
        // Conservative default: no shared mount visible to v2 yet.
        // Lima has writeable mounts under /Users (macOS) and /home
        // (when configured); honoring those needs reading the
        // instance's lima.yaml. Out-of-scope for the trait default;
        // backends with mount metadata can override via a wrapper.
        let _ = ctx_path;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_ops::testing::MockBackend;

    fn ctx(stdout: &str) -> SandboxContext {
        let backend: Arc<dyn SandboxBackend> = Arc::new(
            MockBackend::new("fake").with_stdout(stdout)
        );
        SandboxContext::new(backend, BackendKind::Lima, "test".into())
    }

    #[tokio::test]
    async fn exec_returns_stdout() {
        let c = ctx("hello\n");
        let out = c.exec(&["echo", "hello"]).await.unwrap();
        assert!(out.contains("hello"));
    }

    #[tokio::test]
    async fn id_and_kind() {
        let c = ctx("");
        assert_eq!(c.id(), "lima:test");
        match c.kind() {
            ContextKind::Sandbox { backend, instance } => {
                assert!(matches!(backend, BackendKind::Lima));
                assert_eq!(instance, "test");
            }
            _ => panic!("expected sandbox kind"),
        }
    }
}
