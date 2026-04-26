use async_trait::async_trait;

use crate::common::{OpsError, ProgressSink};

use super::types::{Component, NativeDoctorReport, NativeStatus, VersionSpec};

#[async_trait]
pub trait NativeOps: Send + Sync {
    async fn status(&self) -> Result<NativeStatus, OpsError>;
    async fn list_components(&self) -> Result<Vec<Component>, OpsError>;
    async fn doctor(&self) -> Result<NativeDoctorReport, OpsError>;

    async fn repair(&self, issue_ids: &[String], progress: ProgressSink)
        -> Result<(), OpsError>;
    async fn upgrade_node(&self, target: VersionSpec, progress: ProgressSink)
        -> Result<(), OpsError>;
    async fn upgrade_git(&self, target: VersionSpec, progress: ProgressSink)
        -> Result<(), OpsError>;
    async fn reinstall_component(&self, name: &str, progress: ProgressSink)
        -> Result<(), OpsError>;
}
