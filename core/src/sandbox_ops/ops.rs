//! `SandboxOps` trait.

use async_trait::async_trait;

use crate::common::{CancellationToken, OpsError, ProgressSink};

use super::types::{
    BackendKind, PortRule, ResourceStats, SandboxCaps, SandboxDoctorReport, SandboxStatus,
};

#[async_trait]
pub trait SandboxOps: Send + Sync {
    fn backend_kind(&self) -> BackendKind;
    fn capabilities(&self) -> SandboxCaps;
    fn instance_name(&self) -> &str;

    // ——— Lifecycle ———
    async fn status(&self) -> Result<SandboxStatus, OpsError>;
    async fn start(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>;
    async fn stop(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>;
    async fn restart(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>;

    // ——— Ports ———
    async fn list_ports(&self) -> Result<Vec<PortRule>, OpsError>;
    async fn add_port(&self, host: u16, guest: u16) -> Result<(), OpsError>;
    async fn remove_port(&self, host: u16) -> Result<(), OpsError>;

    // ——— Doctor / repair ———
    async fn doctor(&self) -> Result<SandboxDoctorReport, OpsError>;
    async fn repair(&self, issue_ids: &[String], progress: ProgressSink)
        -> Result<(), OpsError>;

    // ——— Monitoring ———
    async fn stats(&self) -> Result<ResourceStats, OpsError>;
    async fn dump_logs(&self, tail: Option<u32>) -> Result<String, OpsError>;

    /// Rename the underlying VM/container. Default impl bails — only
    /// backends with native rename support (Lima via `limactl rename`)
    /// override this. WSL/Podman would have to destroy+recreate.
    async fn rename(&self, _new_name: &str) -> Result<(), OpsError> {
        Err(OpsError::Other(anyhow::anyhow!(
            "rename: backend does not support in-place rename — destroy + recreate required"
        )))
    }

    /// Resize the VM's primary disk in place. Default impl bails —
    /// only Lima supports it (`limactl disk resize`). WSL/Podman bail.
    async fn resize_disk(&self, _new_gb: u32) -> Result<(), OpsError> {
        Err(OpsError::Other(anyhow::anyhow!(
            "disk resize: backend does not support in-place disk resize"
        )))
    }
}
