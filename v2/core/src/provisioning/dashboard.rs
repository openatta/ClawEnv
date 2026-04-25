//! Hermes-specific dashboard pre-build (P1-f).
//!
//! Lifted from v1 `core/src/manager/install.rs:597-647`. Runs after
//! the claw is installed but before the install pipeline records the
//! instance, so the user's first "Open Control Panel" click doesn't
//! stall behind a 2-3 minute `npm install + npm run build`.
//!
//! Steps inside the VM:
//!   1. Write ~/.<id>/.env API_SERVER_ENABLED=true (idempotent)
//!   2. pip install fastapi uvicorn — workaround for hermes-agent#9569
//!      (uv.lock bug in [web] extra)
//!   3. `sudo chown -R $UID:$GID /opt/<id>` — clone happened as root
//!      under sudo, dashboard runs as sandbox user
//!   4. `cd /opt/<id>/web && npm install && npm run build` —
//!      deliberately best-effort: failures don't fail install,
//!      first-launch will rebuild.

use std::sync::Arc;

use crate::claw_ops::{ClawProvisioning, PackageManager};
use crate::common::{OpsError, ProgressSink};
use crate::sandbox_backend::SandboxBackend;

/// Pre-build the dashboard if this claw has one AND uses GitPip
/// (Hermes today). No-op for everything else. Best-effort: build
/// failures are logged but don't propagate.
pub async fn pre_build_dashboard(
    backend: &Arc<dyn SandboxBackend>,
    provisioning: &dyn ClawProvisioning,
    progress: &ProgressSink,
) -> Result<(), OpsError> {
    if !provisioning.has_dashboard() {
        return Ok(());
    }
    if !matches!(provisioning.package_manager(), PackageManager::GitPip { .. }) {
        return Ok(());
    }
    let id = provisioning.id();

    // 1. API_SERVER_ENABLED env file (idempotent via grep guard).
    progress.info("dashboard", "Configuring API Server").await;
    let env_setup = format!(
        "mkdir -p ~/.{id}; \
         grep -q 'API_SERVER_ENABLED' ~/.{id}/.env 2>/dev/null \
         || printf 'API_SERVER_ENABLED=true\\nAPI_SERVER_KEY=clawenv-local\\n' >> ~/.{id}/.env"
    );
    backend.exec_argv(&["sh", "-c", &env_setup]).await
        .map_err(OpsError::Other)?;

    // 2. fastapi + uvicorn workaround for hermes-agent#9569
    //    (uv.lock resolver bug in [web] extra, silent dashboard fail).
    progress.info("dashboard", "Installing fastapi + uvicorn (workaround)").await;
    let _ = backend.exec_argv(&[
        "sh", "-c",
        "pip install --break-system-packages fastapi 'uvicorn[standard]' 2>/dev/null || true"
    ]).await;

    // 3. chown /opt/<id> from root (clone) to sandbox user (dashboard).
    progress.info("dashboard", "Fixing /opt ownership").await;
    let chown = format!(
        "sudo chown -R $(id -u):$(id -g) /opt/{id} 2>/dev/null || \
         chown -R $(id -u):$(id -g) /opt/{id}"
    );
    backend.exec_argv(&["sh", "-c", &chown]).await
        .map_err(OpsError::Other)?;

    // 4. npm install + npm run build — best-effort, swallow errors.
    progress.info("dashboard", "Pre-building Web UI (~2 min, best-effort)").await;
    let build = format!(
        "cd /opt/{id}/web && npm install --no-audit --no-fund --loglevel=error \
         && npm run build"
    );
    if let Err(e) = backend.exec_argv(&["sh", "-c", &build]).await {
        progress.info(
            "dashboard",
            format!("Web UI pre-build skipped (will retry at first launch): {e}"),
        ).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claw_ops::{HermesProvisioning, OpenClawProvisioning};
    use crate::sandbox_ops::testing::MockBackend;

    #[tokio::test]
    async fn no_op_for_openclaw() {
        // OpenClaw has no separate dashboard.
        let mock = Arc::new(MockBackend::new("fake"));
        let backend: Arc<dyn SandboxBackend> = mock.clone();
        let p = OpenClawProvisioning;
        pre_build_dashboard(&backend, &p, &ProgressSink::noop()).await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        assert!(log.is_empty(), "openclaw should not exec any pre-build cmds");
    }

    #[tokio::test]
    async fn runs_4_steps_for_hermes() {
        let mock = Arc::new(MockBackend::new("fake"));
        let backend: Arc<dyn SandboxBackend> = mock.clone();
        let p = HermesProvisioning;
        pre_build_dashboard(&backend, &p, &ProgressSink::noop()).await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        assert_eq!(log.len(), 4, "expected env+fastapi+chown+build, got {log:?}");
        assert!(log[0].contains(".hermes/.env"), "step 1 env file: {}", log[0]);
        assert!(log[1].contains("fastapi"), "step 2 fastapi: {}", log[1]);
        assert!(log[2].contains("chown"), "step 3 chown: {}", log[2]);
        assert!(log[3].contains("npm run build"), "step 4 build: {}", log[3]);
    }
}
