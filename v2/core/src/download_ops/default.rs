//! `CatalogBackedDownloadOps` — the production DownloadOps impl.
//!
//! Fetch algorithm mirrors v1's `platform::download::download_with_progress`:
//! - connect timeout 15s
//! - chunk stall 60s
//! - throughput floor: 256 KB in 30s
//! - sha256 verify
//! - 2 total attempts with exponential backoff
//!
//! Cache layout:
//!   <cache_root>/<name>/<version>-<os>-<arch>.<ext>

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::common::{CancellationToken, DownloadError, OpsError, ProgressSink};
use crate::sandbox_ops::Severity;

use super::catalog::DownloadCatalog;
use super::ops::DownloadOps;
use super::types::{
    ArtifactSpec, CachedItem, ConnectivityReport, DownloadDoctorIssue, DownloadDoctorReport,
    FetchReport, HostKey, PlatformKey, PruneReport,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CHUNK_STALL: Duration = Duration::from_secs(60);
const MIN_BYTES_BY_DEADLINE: u64 = 256 * 1024;
const MIN_THROUGHPUT_DEADLINE: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS: u32 = 2;
const PROGRESS_BYTES: u64 = 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

pub struct CatalogBackedDownloadOps {
    catalog: DownloadCatalog,
    cache_root: PathBuf,
    platform: PlatformKey,
}

impl CatalogBackedDownloadOps {
    pub fn new(catalog: DownloadCatalog, cache_root: PathBuf, platform: PlatformKey) -> Self {
        Self { catalog, cache_root, platform }
    }

    /// Default factory using v2's paths + current platform + builtin catalog.
    pub fn with_defaults() -> Self {
        Self::new(
            DownloadCatalog::builtin(),
            crate::paths::v2_cache_root(),
            PlatformKey::current(),
        )
    }

    pub fn platform(&self) -> &PlatformKey { &self.platform }

    fn artifact_cache_path(&self, spec: &ArtifactSpec) -> PathBuf {
        self.cache_root.join(&spec.name).join(spec.cache_filename())
    }

    /// Single-attempt streaming download with stall detection + sha256 verify.
    async fn fetch_once(
        &self,
        spec: &ArtifactSpec,
        dest: &Path,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<FetchReport, DownloadError> {
        let started = Instant::now();

        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()?;
        progress.at(0, "download", format!("Connecting to {}", spec.url)).await;

        let resp = client.get(&spec.url).send().await?;
        if !resp.status().is_success() {
            return Err(DownloadError::Http(resp.error_for_status().unwrap_err()));
        }
        let total = resp.content_length();

        let mut file = tokio::fs::File::create(dest).await?;
        let mut hasher = Sha256::new();
        let t0 = Instant::now();
        let mut downloaded: u64 = 0;
        let mut last_emit_bytes: u64 = 0;
        let mut last_emit_at = t0;
        let mut stream = resp.bytes_stream();

        loop {
            let chunk_fut = stream.next();
            let next = tokio::select! {
                _ = cancel.cancelled() => {
                    // Clean up partial file on cancel
                    drop(file);
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(DownloadError::Io(std::io::Error::new(
                        std::io::ErrorKind::Interrupted, "cancelled",
                    )));
                }
                r = tokio::time::timeout(CHUNK_STALL, chunk_fut) => r,
            };
            match next {
                Err(_) => {
                    drop(file);
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(DownloadError::Stalled {
                        url: spec.url.clone(),
                        seconds: CHUNK_STALL.as_secs(),
                    });
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    drop(file);
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err(DownloadError::Http(e));
                }
                Ok(Some(Ok(bytes))) => {
                    hasher.update(&bytes);
                    file.write_all(&bytes).await?;
                    downloaded += bytes.len() as u64;

                    if t0.elapsed() >= MIN_THROUGHPUT_DEADLINE
                        && downloaded < MIN_BYTES_BY_DEADLINE
                    {
                        drop(file);
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(DownloadError::SlowThroughput {
                            url: spec.url.clone(),
                            bytes: downloaded,
                            seconds: MIN_THROUGHPUT_DEADLINE.as_secs(),
                        });
                    }

                    let since_bytes = downloaded - last_emit_bytes;
                    if since_bytes >= PROGRESS_BYTES
                        || last_emit_at.elapsed() >= PROGRESS_INTERVAL
                    {
                        let pct = match total {
                            Some(t) if t > 0 => ((downloaded * 100) / t).min(99) as u8,
                            _ => 0,
                        };
                        progress.at(pct, "download",
                            format!("{} / {} bytes",
                                downloaded,
                                total.map(|t| t.to_string()).unwrap_or_else(|| "?".into()))
                        ).await;
                        last_emit_bytes = downloaded;
                        last_emit_at = Instant::now();
                    }
                }
            }
        }

        file.flush().await?;
        drop(file);

        // sha256 verify (if expected provided)
        let mut verified = false;
        if let Some(expected) = &spec.sha256 {
            let got = hex::encode(hasher.finalize());
            if &got != expected {
                let _ = tokio::fs::remove_file(dest).await;
                return Err(DownloadError::ChecksumMismatch {
                    expected: expected.clone(),
                    got,
                });
            }
            verified = true;
        }

        progress.at(100, "download", format!("Downloaded {} bytes", downloaded)).await;

        Ok(FetchReport {
            path: dest.to_path_buf(),
            bytes: downloaded,
            from_cache: false,
            verified,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn fetch_with_retries(
        &self,
        spec: &ArtifactSpec,
        dest: &Path,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<FetchReport, DownloadError> {
        let mut last_err: Option<DownloadError> = None;
        for attempt in 1..=MAX_ATTEMPTS {
            match self.fetch_once(spec, dest, progress, cancel).await {
                Ok(r) => return Ok(r),
                Err(e) => {
                    tracing::warn!("fetch {} attempt {}/{}: {}", spec.name, attempt, MAX_ATTEMPTS, e);
                    last_err = Some(e);
                    if attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(Duration::from_secs(attempt as u64 * 2)).await;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(||
            DownloadError::Io(std::io::Error::other("no attempt completed"))
        ))
    }

    async fn compute_sha256(path: &Path) -> std::io::Result<String> {
        use tokio::io::AsyncReadExt;
        let mut f = tokio::fs::File::open(path).await?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf).await?;
            if n == 0 { break; }
            hasher.update(&buf[..n]);
        }
        Ok(hex::encode(hasher.finalize()))
    }
}

#[async_trait]
impl DownloadOps for CatalogBackedDownloadOps {
    fn catalog(&self) -> &DownloadCatalog { &self.catalog }

    fn find(&self, name: &str, version: Option<&str>) -> Option<&ArtifactSpec> {
        self.catalog.find(name, version, &self.platform)
    }

    fn cache_root(&self) -> &Path { &self.cache_root }

    async fn list_cached(&self) -> Result<Vec<CachedItem>, OpsError> {
        let mut items = Vec::new();
        if !self.cache_root.exists() {
            return Ok(items);
        }
        let mut entries = tokio::fs::read_dir(&self.cache_root).await?;
        while let Some(e) = entries.next_entry().await? {
            let md = e.metadata().await?;
            if !md.is_dir() { continue; }
            let name = e.file_name().to_string_lossy().into_owned();
            let mut sub = tokio::fs::read_dir(e.path()).await?;
            while let Some(f) = sub.next_entry().await? {
                let fmd = f.metadata().await?;
                if !fmd.is_file() { continue; }
                let fname = f.file_name().to_string_lossy().into_owned();
                let (version, os, arch) = parse_cache_filename(&fname);
                items.push(CachedItem {
                    name: name.clone(),
                    version,
                    platform: PlatformKey { os, arch },
                    path: f.path(),
                    size_bytes: fmd.len(),
                    sha256: None,
                });
            }
        }
        Ok(items)
    }

    async fn verify_cached(&self, item: &CachedItem) -> Result<bool, OpsError> {
        let expected = self.catalog.find(&item.name, Some(&item.version), &item.platform)
            .and_then(|a| a.sha256.as_ref());
        let Some(expected) = expected else { return Ok(true); };  // nothing to verify
        let got = Self::compute_sha256(&item.path).await?;
        Ok(&got == expected)
    }

    async fn prune_cache(&self, keep_per_artifact: usize) -> Result<PruneReport, OpsError> {
        let mut by_name: std::collections::HashMap<String, Vec<CachedItem>> = Default::default();
        for it in self.list_cached().await? {
            by_name.entry(it.name.clone()).or_default().push(it);
        }
        let mut removed = Vec::new();
        let mut freed = 0u64;
        for (_, mut items) in by_name {
            items.sort_by_key(|i| i.version.clone());
            items.reverse();
            for old in items.into_iter().skip(keep_per_artifact) {
                freed += old.size_bytes;
                tokio::fs::remove_file(&old.path).await?;
                removed.push(old.path);
            }
        }
        Ok(PruneReport { removed, freed_bytes: freed })
    }

    async fn fetch(
        &self, name: &str, version: Option<&str>,
        progress: ProgressSink, cancel: CancellationToken,
    ) -> Result<PathBuf, OpsError> {
        let spec = self.find(name, version)
            .ok_or_else(|| OpsError::Download(DownloadError::NotInCatalog {
                name: name.into()
            }))?;
        let dest = self.artifact_cache_path(spec);

        // Cache hit?
        if dest.exists() {
            if let Some(expected) = &spec.sha256 {
                let got = Self::compute_sha256(&dest).await?;
                if &got == expected {
                    progress.at(100, "cache", format!("Hit: {}", dest.display())).await;
                    return Ok(dest);
                } else {
                    tokio::fs::remove_file(&dest).await?;
                }
            } else {
                progress.at(100, "cache", format!("Hit (unverified): {}", dest.display())).await;
                return Ok(dest);
            }
        }

        let spec_clone = spec.clone();
        self.fetch_with_retries(&spec_clone, &dest, &progress, &cancel).await?;
        Ok(dest)
    }

    async fn fetch_to(
        &self, name: &str, version: Option<&str>, dest: &Path,
        progress: ProgressSink, cancel: CancellationToken,
    ) -> Result<FetchReport, OpsError> {
        let spec = self.find(name, version)
            .ok_or_else(|| OpsError::Download(DownloadError::NotInCatalog {
                name: name.into()
            }))?
            .clone();
        Ok(self.fetch_with_retries(&spec, dest, &progress, &cancel).await?)
    }

    async fn doctor(&self) -> Result<DownloadDoctorReport, OpsError> {
        let mut issues = Vec::new();
        if !self.cache_root.exists() {
            issues.push(DownloadDoctorIssue {
                id: "cache-missing".into(),
                severity: Severity::Info,
                message: format!("Cache directory does not yet exist: {}", self.cache_root.display()),
                repair_hint: Some("Will be created on first fetch".into()),
            });
        }
        // Catalog sanity
        let matching = self.catalog.by_platform(&self.platform);
        if matching.is_empty() {
            issues.push(DownloadDoctorIssue {
                id: "catalog-empty-for-platform".into(),
                severity: Severity::Warning,
                message: format!(
                    "No catalog entries for current platform {}/{}",
                    self.platform.os, self.platform.arch
                ),
                repair_hint: None,
            });
        }
        Ok(DownloadDoctorReport {
            issues,
            checked_at: Utc::now().to_rfc3339(),
        })
    }

    async fn check_connectivity(&self) -> Result<ConnectivityReport, OpsError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| OpsError::Download(DownloadError::Http(e)))?;

        let mut hosts_seen: std::collections::HashSet<String> = Default::default();
        let mut hosts = Vec::new();

        for artifact in self.catalog.artifacts() {
            let Some(host) = reqwest::Url::parse(&artifact.url).ok()
                .and_then(|u| u.host_str().map(String::from)) else { continue };
            if !hosts_seen.insert(host.clone()) { continue; }

            let t0 = Instant::now();
            let (reachable, status, note) = match client.head(&artifact.url).send().await {
                Ok(resp) => (true, Some(resp.status().as_u16()), None),
                Err(e) => (false, None, Some(format!("{}", e))),
            };
            hosts.push(HostKey {
                host,
                reachable,
                tls_handshake_ms: if reachable { Some(t0.elapsed().as_millis() as u64) } else { None },
                http_status: status,
                note,
            });
        }

        Ok(ConnectivityReport {
            hosts,
            http_proxy_env: std::env::var("HTTP_PROXY").or_else(|_| std::env::var("http_proxy")).ok(),
            https_proxy_env: std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("https_proxy")).ok(),
            no_proxy_env: std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).ok(),
            checked_at: Utc::now().to_rfc3339(),
        })
    }
}

/// Parse a cache filename `<version>-<os>-<arch>.<ext>` into its parts.
///
/// Version strings may contain dots (`22.16.0`), so splitting on `.` is
/// wrong; we strip the known archive suffix first, then split from the
/// right on `-` to pull off (arch, os, version).
pub(crate) fn parse_cache_filename(fname: &str) -> (String, String, String) {
    let stem = trim_archive_ext(fname);
    // rsplit to take from the right: ["arm64", "macos", "22.16.0"]
    let parts: Vec<&str> = stem.rsplitn(3, '-').collect();
    match parts.as_slice() {
        [arch, os, version] => (version.to_string(), os.to_string(), arch.to_string()),
        _ => (stem.to_string(), "unknown".into(), "unknown".into()),
    }
}

fn trim_archive_ext(fname: &str) -> String {
    for ext in [".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".zip", ".rootfs.tar.gz"] {
        if let Some(stripped) = fname.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    fname.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tiny_catalog() -> DownloadCatalog {
        DownloadCatalog::from_toml_str(r#"
            [[artifact]]
            name = "demo"
            version = "1.0.0"
            os = "macos"
            arch = "arm64"
            url = "https://example.invalid/demo.tar.gz"
            kind = "tarball"
        "#).unwrap()
    }

    #[tokio::test]
    async fn list_cached_empty_when_missing() {
        let tmp = TempDir::new().unwrap();
        let ops = CatalogBackedDownloadOps::new(
            DownloadCatalog::empty(),
            tmp.path().to_path_buf(),
            PlatformKey { os: "macos".into(), arch: "arm64".into() },
        );
        let items = ops.list_cached().await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn list_cached_scans_layout() {
        let tmp = TempDir::new().unwrap();
        // create cache_root/node/22.12.0-macos-arm64.tar.gz
        let sub = tmp.path().join("node");
        tokio::fs::create_dir_all(&sub).await.unwrap();
        tokio::fs::write(sub.join("22.12.0-macos-arm64.tar.gz"), b"test data").await.unwrap();

        let ops = CatalogBackedDownloadOps::new(
            DownloadCatalog::empty(),
            tmp.path().to_path_buf(),
            PlatformKey { os: "macos".into(), arch: "arm64".into() },
        );
        let items = ops.list_cached().await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "node");
        assert_eq!(items[0].version, "22.12.0");
        assert_eq!(items[0].platform.os, "macos");
        assert_eq!(items[0].platform.arch, "arm64");
    }

    #[test]
    fn parse_cache_filename_variants() {
        assert_eq!(
            parse_cache_filename("22.16.0-macos-arm64.tar.gz"),
            ("22.16.0".into(), "macos".into(), "arm64".into())
        );
        assert_eq!(
            parse_cache_filename("22.16.0-linux-x86_64.tar.xz"),
            ("22.16.0".into(), "linux".into(), "x86_64".into())
        );
        assert_eq!(
            parse_cache_filename("2.49.0-windows.1-windows-arm64.zip"),
            ("2.49.0-windows.1".into(), "windows".into(), "arm64".into())
        );
    }

    #[tokio::test]
    async fn find_returns_matching_artifact() {
        let tmp = TempDir::new().unwrap();
        let ops = CatalogBackedDownloadOps::new(
            tiny_catalog(),
            tmp.path().to_path_buf(),
            PlatformKey { os: "macos".into(), arch: "arm64".into() },
        );
        assert!(ops.find("demo", None).is_some());
        assert!(ops.find("missing", None).is_none());
    }

    #[tokio::test]
    async fn doctor_warns_when_platform_unmatched() {
        let tmp = TempDir::new().unwrap();
        let ops = CatalogBackedDownloadOps::new(
            tiny_catalog(),
            tmp.path().to_path_buf(),
            PlatformKey { os: "linux".into(), arch: "x86_64".into() }, // no matches
        );
        let r = ops.doctor().await.unwrap();
        assert!(r.issues.iter().any(|i| i.id == "catalog-empty-for-platform"));
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_artifact() {
        let tmp = TempDir::new().unwrap();
        let ops = CatalogBackedDownloadOps::new(
            tiny_catalog(),
            tmp.path().to_path_buf(),
            PlatformKey { os: "macos".into(), arch: "arm64".into() },
        );
        let err = ops.fetch("nope", None, ProgressSink::noop(), CancellationToken::new())
            .await.unwrap_err();
        match err {
            OpsError::Download(DownloadError::NotInCatalog { name }) => {
                assert_eq!(name, "nope");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
