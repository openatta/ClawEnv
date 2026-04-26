use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::sandbox_ops::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    Binary,
    Tarball,
    Zip,
    Rootfs,
    OciImage,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlatformKey {
    pub os: String,       // "macos" | "linux" | "windows"
    pub arch: String,     // "arm64" | "x86_64"
}

impl PlatformKey {
    pub fn current() -> Self {
        let os = if cfg!(target_os = "macos") { "macos" }
                 else if cfg!(target_os = "windows") { "windows" }
                 else { "linux" };
        let arch = if cfg!(target_arch = "aarch64") { "arm64" }
                   else { "x86_64" };
        Self { os: os.into(), arch: arch.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSpec {
    pub name: String,
    pub version: String,
    #[serde(flatten)]
    pub platform: PlatformKey,
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_hint: Option<u64>,
    pub kind: ArtifactKind,
}

impl ArtifactSpec {
    /// Suggested filename inside cache (`<version>-<os>-<arch>.<ext>`).
    pub fn cache_filename(&self) -> String {
        let ext = match self.kind {
            ArtifactKind::Tarball => {
                if self.url.ends_with(".tar.xz") { "tar.xz" }
                else if self.url.ends_with(".tar.bz2") { "tar.bz2" }
                else { "tar.gz" }
            }
            ArtifactKind::Zip => "zip",
            ArtifactKind::Binary => "bin",
            ArtifactKind::Rootfs => "rootfs.tar.gz",
            ArtifactKind::OciImage => "oci",
        };
        format!("{}-{}-{}.{}", self.version, self.platform.os, self.platform.arch, ext)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedItem {
    pub name: String,
    pub version: String,
    pub platform: PlatformKey,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub from_cache: bool,
    pub verified: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneReport {
    pub removed: Vec<PathBuf>,
    pub freed_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostKey {
    pub host: String,              // nodejs.org
    pub reachable: bool,
    pub tls_handshake_ms: Option<u64>,
    pub http_status: Option<u16>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityReport {
    pub hosts: Vec<HostKey>,
    pub http_proxy_env: Option<String>,
    pub https_proxy_env: Option<String>,
    pub no_proxy_env: Option<String>,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadDoctorIssue {
    pub id: String,
    pub severity: Severity,
    pub message: String,
    pub repair_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadDoctorReport {
    pub issues: Vec<DownloadDoctorIssue>,
    pub checked_at: String,
}

impl DownloadDoctorReport {
    pub fn healthy(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Error)
    }
}
