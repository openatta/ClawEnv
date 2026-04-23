//! DownloadOps — artifact catalog + cache + resumable fetch with stall detection.

pub mod types;
pub mod catalog;
pub mod ops;
pub mod default;

pub use types::{
    ArtifactKind, ArtifactSpec, CachedItem, ConnectivityReport, DownloadDoctorIssue,
    DownloadDoctorReport, FetchReport, HostKey, PlatformKey, PruneReport,
};
pub use catalog::DownloadCatalog;
pub use ops::DownloadOps;
pub use default::CatalogBackedDownloadOps;
