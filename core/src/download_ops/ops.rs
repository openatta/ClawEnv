use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::common::{CancellationToken, OpsError, ProgressSink};

use super::catalog::DownloadCatalog;
use super::types::{
    ArtifactSpec, CachedItem, ConnectivityReport, DownloadDoctorReport, FetchReport,
};

#[async_trait]
pub trait DownloadOps: Send + Sync {
    // Catalog
    fn catalog(&self) -> &DownloadCatalog;
    fn list_artifacts(&self) -> Vec<&ArtifactSpec> {
        self.catalog().artifacts().iter().collect()
    }
    fn find(&self, name: &str, version: Option<&str>) -> Option<&ArtifactSpec>;

    // Cache
    fn cache_root(&self) -> &Path;
    async fn list_cached(&self) -> Result<Vec<CachedItem>, OpsError>;
    async fn verify_cached(&self, item: &CachedItem) -> Result<bool, OpsError>;
    async fn prune_cache(&self, keep_per_artifact: usize)
        -> Result<super::types::PruneReport, OpsError>;

    // Fetch
    async fn fetch(
        &self, name: &str, version: Option<&str>,
        progress: ProgressSink, cancel: CancellationToken,
    ) -> Result<PathBuf, OpsError>;
    async fn fetch_to(
        &self, name: &str, version: Option<&str>, dest: &Path,
        progress: ProgressSink, cancel: CancellationToken,
    ) -> Result<FetchReport, OpsError>;

    // Diagnostics
    async fn doctor(&self) -> Result<DownloadDoctorReport, OpsError>;
    async fn check_connectivity(&self) -> Result<ConnectivityReport, OpsError>;
}
