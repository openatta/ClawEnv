//! Shared repair dispatch for the three sandbox backends.
//!
//! Only handles issue ids that `doctor()` actually emits today. Unknown ids
//! return `OpsError::Unsupported` so callers get a clear signal instead of a
//! silent noop.

use std::sync::Arc;

use crate::common::{OpsError, ProgressSink};
use crate::sandbox_backend::SandboxBackend;

/// Run the repair recipe for each requested issue.
///
/// Repairs run in the order the ids are given. The first failure short-circuits.
pub(crate) async fn dispatch_repair(
    backend: &Arc<dyn SandboxBackend>,
    issue_ids: &[String],
    progress: &ProgressSink,
) -> Result<(), OpsError> {
    if issue_ids.is_empty() {
        return Err(OpsError::unsupported(
            "repair",
            "no issue ids given; pass at least one",
        ));
    }
    for id in issue_ids {
        match id.as_str() {
            "vm-not-running" | "vm-stopped" => {
                progress
                    .info("repair", format!("starting {}", backend.name()))
                    .await;
                backend.start().await.map_err(OpsError::Other)?;
            }
            other => {
                return Err(OpsError::unsupported(
                    "repair",
                    format!("unknown issue id: {other}"),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::sandbox_backend::ResourceStats;

    struct FakeBackend {
        name: &'static str,
        start_calls: AtomicU32,
        start_err: bool,
    }

    impl FakeBackend {
        fn new(name: &'static str, start_err: bool) -> Self {
            Self {
                name,
                start_calls: AtomicU32::new(0),
                start_err,
            }
        }
    }

    #[async_trait]
    impl SandboxBackend for FakeBackend {
        fn name(&self) -> &str { self.name }
        fn instance(&self) -> &str { "test" }
        async fn is_available(&self) -> anyhow::Result<bool> { Ok(false) }
        async fn create(&self, _opts: &crate::provisioning::CreateOpts) -> anyhow::Result<()> {
            Ok(())
        }
        async fn destroy(&self) -> anyhow::Result<()> { Ok(()) }
        async fn start(&self) -> anyhow::Result<()> {
            self.start_calls.fetch_add(1, Ordering::SeqCst);
            if self.start_err {
                anyhow::bail!("boom");
            }
            Ok(())
        }
        async fn stop(&self) -> anyhow::Result<()> { Ok(()) }
        async fn exec(&self, _cmd: &str) -> anyhow::Result<String> { Ok(String::new()) }
        async fn stats(&self) -> anyhow::Result<ResourceStats> { Ok(ResourceStats::default()) }
        async fn edit_port_forwards(&self, _forwards: &[(u16, u16)]) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn vm_not_running_calls_backend_start() {
        let fake = Arc::new(FakeBackend::new("fake", false));
        let backend: Arc<dyn SandboxBackend> = fake.clone();
        dispatch_repair(
            &backend,
            &["vm-not-running".into()],
            &ProgressSink::noop(),
        )
        .await
        .unwrap();
        assert_eq!(fake.start_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn vm_stopped_is_same_recipe_as_not_running() {
        let fake = Arc::new(FakeBackend::new("fake", false));
        let backend: Arc<dyn SandboxBackend> = fake.clone();
        dispatch_repair(&backend, &["vm-stopped".into()], &ProgressSink::noop())
            .await
            .unwrap();
        assert_eq!(fake.start_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unknown_issue_id_returns_unsupported() {
        let fake = Arc::new(FakeBackend::new("fake", false));
        let backend: Arc<dyn SandboxBackend> = fake;
        let err = dispatch_repair(
            &backend,
            &["no-such-issue".into()],
            &ProgressSink::noop(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, OpsError::Unsupported { .. }));
    }

    #[tokio::test]
    async fn empty_issue_ids_rejected() {
        let fake = Arc::new(FakeBackend::new("fake", false));
        let backend: Arc<dyn SandboxBackend> = fake;
        let err = dispatch_repair(&backend, &[], &ProgressSink::noop())
            .await
            .unwrap_err();
        assert!(matches!(err, OpsError::Unsupported { .. }));
    }

    #[tokio::test]
    async fn backend_start_failure_surfaces_as_other() {
        let fake = Arc::new(FakeBackend::new("fake", true));
        let backend: Arc<dyn SandboxBackend> = fake;
        let err = dispatch_repair(
            &backend,
            &["vm-not-running".into()],
            &ProgressSink::noop(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, OpsError::Other(_)));
    }

    #[tokio::test]
    async fn multiple_ids_each_invoke_recipe() {
        let fake = Arc::new(FakeBackend::new("fake", false));
        let backend: Arc<dyn SandboxBackend> = fake.clone();
        dispatch_repair(
            &backend,
            &["vm-not-running".into(), "vm-stopped".into()],
            &ProgressSink::noop(),
        )
        .await
        .unwrap();
        // Both recipes hit start(); idempotent on the backend side.
        assert_eq!(fake.start_calls.load(Ordering::SeqCst), 2);
    }
}
