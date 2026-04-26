//! Post-boot verify gate — runs immediately after `SandboxBackend::start()`
//! returns OK, before any provisioning step that would fail confusingly
//! on a half-ready VM.
//!
//! v0.2.12 lesson (lifted from v1 CHANGELOG): a freshly booted Lima VM
//! reports "Running" while several subsystems are still warming up:
//!
//! - SSH ControlMaster master socket may not have completed its first
//!   handshake → `exec_argv` returns exit 255 with
//!   `kex_exchange_identification: read: Connection reset` once.
//! - `/etc/resolv.conf` may be empty for ~500ms while cloud-init's
//!   DHCP-renewed copy is being written → `apk add` fails on DNS.
//! - The rootfs may be read-only for ~200ms until `mount -o remount,rw`
//!   finishes → `apk add` fails to write the lock file.
//!
//! Three distinct probes catch each failure mode separately, so when
//! one fails the error message points at the actual problem, not at
//! whatever apk happens to be doing when the cascade hits. Each probe
//! uses `exec_argv_with_retry` so transient SSH races during the probe
//! itself don't prematurely fail the gate.

use std::sync::Arc;

use crate::common::OpsError;
use crate::sandbox_backend::SandboxBackend;

/// Three-probe post-boot verify gate. Returns Ok when the VM is alive,
/// has working DNS, and has a writeable rootfs. Bails with a structured
/// error pointing at the failed probe otherwise.
///
/// Worst case wall: 3 × (0 + 1 + 3 + 9) = ~39 seconds, only on a VM
/// that's genuinely failing — happy path returns in milliseconds.
pub async fn verify_post_boot(backend: &Arc<dyn SandboxBackend>) -> Result<(), OpsError> {
    // Probe 1 — alive. Does the VM accept exec at all?
    let out = backend
        .exec_argv_with_retry(&["echo", "clawenv-alive"])
        .await
        .map_err(|e| OpsError::Other(anyhow::anyhow!(
            "post-boot probe `alive` exhausted retries: {e}\n\
             hint: VM `Running` but exec is unreachable — check SSH \
             master socket / waagent / WSL service state."
        )))?;
    if !out.contains("clawenv-alive") {
        return Err(OpsError::Other(anyhow::anyhow!(
            "post-boot probe `alive` returned unexpected output: {out:?}"
        )));
    }

    // Probe 2 — DNS. Cloud-init / WSL networking sometimes write an
    // empty resolv.conf during the brief gap between DHCP renew and the
    // first userspace lookup. `getent hosts localhost` doesn't touch
    // upstream DNS but DOES exercise nsswitch.conf + the resolver
    // library, which is what apk/curl use.
    backend
        .exec_argv_with_retry(&["sh", "-c", "getent hosts localhost"])
        .await
        .map_err(|e| OpsError::Other(anyhow::anyhow!(
            "post-boot probe `dns` exhausted retries: {e}\n\
             hint: /etc/resolv.conf may be empty or nsswitch.conf misconfigured \
             — re-run `clawcli sandbox doctor` once VM is fully up."
        )))?;

    // Probe 3 — fs. apk add / npm install both write to /tmp and the
    // package db; if rootfs is still mounted read-only post-boot, every
    // package install fails with a confusing `Permission denied`. Touch
    // a sentinel file in /tmp and immediately remove it.
    backend
        .exec_argv_with_retry(&[
            "sh", "-c",
            "touch /tmp/.clawenv-postboot && rm -f /tmp/.clawenv-postboot",
        ])
        .await
        .map_err(|e| OpsError::Other(anyhow::anyhow!(
            "post-boot probe `fs` exhausted retries: {e}\n\
             hint: rootfs may not be remounted writeable yet \
             — wait a few seconds and retry, or check `dmesg` for fs errors."
        )))?;

    tracing::info!(target: "clawenv::postboot", "all 3 post-boot probes OK");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_ops::testing::MockBackend;

    fn arc_mock(out: &str) -> Arc<dyn SandboxBackend> {
        Arc::new(MockBackend::new("fake").with_stdout(out))
    }

    #[tokio::test]
    async fn happy_path_passes_all_three_probes() {
        let backend = arc_mock("clawenv-alive\n");
        verify_post_boot(&backend).await.unwrap();
    }

    #[tokio::test]
    async fn alive_probe_unexpected_output_fails() {
        // VM responded but with garbage — would mean a different shell
        // is intercepting the echo, or the exec channel is corrupting
        // output. Either way we want a hard fail with a clear pointer
        // at the `alive` probe.
        let backend = arc_mock("garbage\n");
        let err = verify_post_boot(&backend).await.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("alive"), "expected alive probe failure, got: {msg}");
        assert!(msg.contains("unexpected output"), "got: {msg}");
    }

    #[tokio::test]
    async fn three_probes_run_in_order() {
        // Each successful probe logs its quoted argv to MockBackend's
        // exec_log. Verify all three ran and in the documented order.
        let mock = MockBackend::new("fake").with_stdout("clawenv-alive\n");
        let mock_arc: Arc<MockBackend> = Arc::new(mock);
        let backend: Arc<dyn SandboxBackend> = mock_arc.clone();
        verify_post_boot(&backend).await.unwrap();
        let log = mock_arc.exec_log.lock().unwrap();
        assert_eq!(log.len(), 3, "expected exactly 3 probes, got {log:?}");
        assert!(log[0].contains("clawenv-alive"), "probe 0 not alive: {log:?}");
        assert!(log[1].contains("getent hosts localhost"), "probe 1 not dns: {log:?}");
        assert!(log[2].contains("clawenv-postboot"), "probe 2 not fs: {log:?}");
    }
}
