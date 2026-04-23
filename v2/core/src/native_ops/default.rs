//! `DefaultNativeOps` — both read-only probes and write operations.
//!
//! Write operations (upgrade_node/git, repair, reinstall) use v2's own
//! `DownloadOps` + `extract::extract_archive` — no v1 install calls.
//! Path computations re-use v1's `clawenv_node_dir()` / `clawenv_git_dir()`
//! so layouts stay compatible between v1 and v2 on the same host.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::paths::{clawenv_git_dir, clawenv_node_dir, clawenv_root};
use crate::common::{CancellationToken, CommandRunner, CommandSpec, OpsError, ProgressSink};
use crate::download_ops::{CatalogBackedDownloadOps, DownloadOps};
use crate::extract::{extract_archive, ExtractError, ExtractOpts};
use crate::runners::LocalProcessRunner;
use crate::sandbox_ops::Severity;

use super::ops::NativeOps;
use super::types::{
    Component, ComponentInfo, NativeDoctorIssue, NativeDoctorReport, NativeStatus, VersionSpec,
};

pub struct DefaultNativeOps {
    runner: LocalProcessRunner,
    downloader: CatalogBackedDownloadOps,
}

impl Default for DefaultNativeOps {
    fn default() -> Self { Self::new() }
}

impl DefaultNativeOps {
    pub fn new() -> Self {
        Self {
            runner: LocalProcessRunner::new(),
            downloader: CatalogBackedDownloadOps::with_defaults(),
        }
    }

    /// Test-only: inject a downloader pointed at a fixture server + custom
    /// paths.
    pub fn with_downloader(downloader: CatalogBackedDownloadOps) -> Self {
        Self { runner: LocalProcessRunner::new(), downloader }
    }

    fn node_binary_path() -> PathBuf {
        // Layout mirrors v1 (dugite/node tarballs ship a top-level wrapper
        // dir; after strip_components=1 the binary sits under `bin/`).
        #[cfg(target_os = "windows")]
        { clawenv_node_dir().join("node.exe") }
        #[cfg(not(target_os = "windows"))]
        { clawenv_node_dir().join("bin").join("node") }
    }

    fn git_binary_path() -> PathBuf {
        // Windows MinGit uses `cmd/git.exe`; unix dugite uses `bin/git`.
        #[cfg(target_os = "windows")]
        { clawenv_git_dir().join("cmd").join("git.exe") }
        #[cfg(not(target_os = "windows"))]
        { clawenv_git_dir().join("bin").join("git") }
    }

    async fn probe_version(&self, bin: &Path) -> Option<String> {
        if !bin.exists() { return None; }
        let spec = CommandSpec::new(bin.to_string_lossy(), ["--version"])
            .with_timeout(Duration::from_secs(5));
        let res = self.runner.exec(spec, CancellationToken::new()).await.ok()?;
        if !res.success() { return None; }
        res.stdout.lines().next().map(|s| s.trim().to_string())
    }

    fn dir_size_bytes(path: &Path) -> u64 {
        fn walk(p: &Path) -> u64 {
            let Ok(md) = std::fs::metadata(p) else { return 0 };
            if md.is_file() { return md.len(); }
            if !md.is_dir() { return 0; }
            let Ok(rd) = std::fs::read_dir(p) else { return 0 };
            let mut total = 0u64;
            for entry in rd.flatten() {
                total += walk(&entry.path());
            }
            total
        }
        walk(path)
    }

    /// Fetch + extract an artifact into `target_dir`.
    ///
    /// - Selects catalog entry by `name` + optional version + current platform.
    /// - Downloads into cache (or hits cache).
    /// - Extracts into a temp dir, then atomically swaps into `target_dir`
    ///   (old dir renamed out of the way first so interrupted upgrades
    ///   don't leave a broken install).
    /// - Strip 1 component (node/dugite/lima all wrap in a top dir).
    ///
    /// Public so integration tests can target a custom `target_dir` without
    /// setting the process-wide `CLAWENV_HOME` env var.
    pub async fn install_component(
        &self,
        name: &str,
        version: VersionSpec,
        target_dir: &Path,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), OpsError> {
        let version_str = match &version {
            VersionSpec::Latest => None,
            VersionSpec::Exact(v) => Some(v.as_str()),
        };

        progress.at(5, "resolve", format!("Resolving {name}")).await;
        let archive_path = self.downloader
            .fetch(name, version_str, progress.clone(), cancel.clone())
            .await?;

        progress.at(70, "extract", format!("Extracting {}", archive_path.display())).await;
        let staging = target_dir.with_extension("staging");
        // Best-effort cleanup of any previous failed staging.
        let _ = tokio::fs::remove_dir_all(&staging).await;

        // Extraction is sync; move off the async reactor.
        let archive_clone = archive_path.clone();
        let staging_clone = staging.clone();
        tokio::task::spawn_blocking(move || -> Result<(), ExtractError> {
            extract_archive(&archive_clone, &staging_clone, &ExtractOpts {
                strip_components: 1,
                clean_dest: true,
            })
        }).await
            .map_err(|e| OpsError::Other(anyhow::anyhow!("extract join: {e}")))?
            .map_err(|e| OpsError::Other(anyhow::anyhow!("extract failed: {e}")))?;

        // Atomic-ish swap: move target_dir → .old, move staging → target_dir.
        progress.at(92, "swap", "Swapping new version into place").await;
        if target_dir.exists() {
            let backup = target_dir.with_extension("old");
            let _ = tokio::fs::remove_dir_all(&backup).await;
            tokio::fs::rename(target_dir, &backup).await?;
        }
        tokio::fs::rename(&staging, target_dir).await?;
        // Best-effort cleanup of backup (don't fail the upgrade on this).
        let backup = target_dir.with_extension("old");
        if backup.exists() {
            let _ = tokio::fs::remove_dir_all(&backup).await;
        }

        progress.at(100, "done", format!("Installed {name}")).await;
        Ok(())
    }
}

#[async_trait]
impl NativeOps for DefaultNativeOps {
    async fn status(&self) -> Result<NativeStatus, OpsError> {
        let home = clawenv_root();
        let home_exists = home.exists();

        let node_path = Self::node_binary_path();
        let node = match self.probe_version(&node_path).await {
            Some(v) => Some(ComponentInfo { version: v, path: node_path.clone(), healthy: true }),
            None if node_path.exists() => Some(ComponentInfo {
                version: "unknown".into(), path: node_path, healthy: false
            }),
            None => None,
        };

        let git_path = Self::git_binary_path();
        let git = match self.probe_version(&git_path).await {
            Some(v) => Some(ComponentInfo { version: v, path: git_path.clone(), healthy: true }),
            None if git_path.exists() => Some(ComponentInfo {
                version: "unknown".into(), path: git_path, healthy: false
            }),
            None => None,
        };

        let total_disk_bytes = if home_exists { Self::dir_size_bytes(&home) } else { 0 };

        Ok(NativeStatus {
            clawenv_home: home,
            home_exists, node, git, total_disk_bytes,
        })
    }

    async fn list_components(&self) -> Result<Vec<Component>, OpsError> {
        let s = self.status().await?;
        let mut out = Vec::new();
        let node_dir = clawenv_node_dir();
        out.push(Component {
            name: "node".into(),
            version: s.node.as_ref().map(|n| n.version.clone()),
            path: s.node.as_ref().map(|n| n.path.clone()),
            healthy: s.node.as_ref().map(|n| n.healthy).unwrap_or(false),
            size_bytes: if node_dir.exists() { Self::dir_size_bytes(&node_dir) } else { 0 },
        });
        let git_dir = clawenv_git_dir();
        out.push(Component {
            name: "git".into(),
            version: s.git.as_ref().map(|g| g.version.clone()),
            path: s.git.as_ref().map(|g| g.path.clone()),
            healthy: s.git.as_ref().map(|g| g.healthy).unwrap_or(false),
            size_bytes: if git_dir.exists() { Self::dir_size_bytes(&git_dir) } else { 0 },
        });
        Ok(out)
    }

    async fn doctor(&self) -> Result<NativeDoctorReport, OpsError> {
        let mut issues = Vec::new();
        let home = clawenv_root();
        if !home.exists() {
            issues.push(NativeDoctorIssue {
                id: "clawenv-home-missing".into(),
                severity: Severity::Warning,
                message: format!("ClawEnv home directory missing: {}", home.display()),
                repair_hint: Some("clawops native repair clawenv-home-missing".into()),
                auto_repairable: true,
            });
        }
        let node_path = Self::node_binary_path();
        if !node_path.exists() {
            issues.push(NativeDoctorIssue {
                id: "node-missing".into(),
                severity: Severity::Error,
                message: format!("node binary missing at {}", node_path.display()),
                repair_hint: Some("clawops native upgrade node".into()),
                auto_repairable: true,
            });
        } else if self.probe_version(&node_path).await.is_none() {
            issues.push(NativeDoctorIssue {
                id: "node-unversionable".into(),
                severity: Severity::Error,
                message: "node exists but `--version` failed".into(),
                repair_hint: Some("clawops native reinstall node".into()),
                auto_repairable: true,
            });
        }
        let git_path = Self::git_binary_path();
        if !git_path.exists() {
            issues.push(NativeDoctorIssue {
                id: "git-missing".into(),
                severity: Severity::Error,
                message: format!("git binary missing at {}", git_path.display()),
                repair_hint: Some("clawops native upgrade git".into()),
                auto_repairable: true,
            });
        } else if self.probe_version(&git_path).await.is_none() {
            issues.push(NativeDoctorIssue {
                id: "git-unversionable".into(),
                severity: Severity::Error,
                message: "git exists but `--version` failed".into(),
                repair_hint: Some("clawops native reinstall git".into()),
                auto_repairable: true,
            });
        }

        // Host-side PATH shadowing: if `node`/`git` on PATH resolves to
        // something other than our clawenv binary, the shell picks up the
        // system one silently. Warning, not error.
        for (id, target, binary) in [
            ("node-path-shadowed", &node_path, "node"),
            ("git-path-shadowed",  &git_path,  "git"),
        ] {
            if !target.exists() { continue; }
            let which_cmd = if cfg!(target_os = "windows") { "where" } else { "which" };
            if let Ok(res) = self.runner.exec(
                CommandSpec::new(which_cmd, [binary]).with_timeout(Duration::from_secs(2)),
                CancellationToken::new(),
            ).await {
                if res.success() {
                    let first = res.stdout.lines().next().unwrap_or("").trim();
                    if !first.is_empty() && Path::new(first) != target.as_path() {
                        issues.push(NativeDoctorIssue {
                            id: id.into(),
                            severity: Severity::Warning,
                            message: format!(
                                "System `{binary}` on PATH is {first}, not our {}",
                                target.display()
                            ),
                            repair_hint: Some(
                                format!("Ensure clawenv's {binary} dir precedes system PATH")
                            ),
                            auto_repairable: false,
                        });
                    }
                }
            }
        }

        // Home writability: can we create a temp file under clawenv_home?
        if home.exists() {
            let probe = home.join(".clawops-writability-probe");
            match std::fs::write(&probe, b"x") {
                Ok(_) => { let _ = std::fs::remove_file(&probe); }
                Err(e) => issues.push(NativeDoctorIssue {
                    id: "home-not-writable".into(),
                    severity: Severity::Error,
                    message: format!("Cannot write to {}: {}", home.display(), e),
                    repair_hint: Some("Check permissions on ~/.clawenv".into()),
                    auto_repairable: false,
                }),
            }
        }

        Ok(NativeDoctorReport {
            issues,
            checked_at: Utc::now().to_rfc3339(),
        })
    }

    async fn repair(&self, issue_ids: &[String], progress: ProgressSink)
        -> Result<(), OpsError>
    {
        for id in issue_ids {
            match id.as_str() {
                "clawenv-home-missing" => {
                    tokio::fs::create_dir_all(clawenv_root()).await?;
                    progress.info("repair", "created clawenv home").await;
                }
                "node-missing" | "node-unversionable" => {
                    self.upgrade_node(VersionSpec::Latest, progress.clone()).await?;
                }
                "git-missing" | "git-unversionable" => {
                    self.upgrade_git(VersionSpec::Latest, progress.clone()).await?;
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

    async fn upgrade_node(&self, target: VersionSpec, progress: ProgressSink)
        -> Result<(), OpsError>
    {
        let target_dir = clawenv_node_dir();
        let cancel = CancellationToken::new();
        self.install_component("node", target, &target_dir, &progress, &cancel).await
    }

    async fn upgrade_git(&self, target: VersionSpec, progress: ProgressSink)
        -> Result<(), OpsError>
    {
        let target_dir = clawenv_git_dir();
        let cancel = CancellationToken::new();
        self.install_component("git", target, &target_dir, &progress, &cancel).await
    }

    async fn reinstall_component(&self, name: &str, progress: ProgressSink)
        -> Result<(), OpsError>
    {
        let (target_dir, art_name) = match name {
            "node" => (clawenv_node_dir(), "node"),
            "git"  => (clawenv_git_dir(), "git"),
            other => return Err(OpsError::not_found(format!("component: {other}"))),
        };
        // Remove then install fresh.
        if target_dir.exists() {
            tokio::fs::remove_dir_all(&target_dir).await?;
        }
        let cancel = CancellationToken::new();
        self.install_component(art_name, VersionSpec::Latest, &target_dir, &progress, &cancel).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_is_well_formed() {
        let ops = DefaultNativeOps::new();
        let s = ops.status().await.unwrap();
        assert!(s.clawenv_home.is_absolute() || !s.clawenv_home.as_os_str().is_empty());
    }

    #[tokio::test]
    async fn list_components_returns_node_and_git() {
        let ops = DefaultNativeOps::new();
        let cs = ops.list_components().await.unwrap();
        let names: Vec<&str> = cs.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"node"));
        assert!(names.contains(&"git"));
    }

    #[tokio::test]
    async fn doctor_never_panics() {
        let ops = DefaultNativeOps::new();
        let r = ops.doctor().await.unwrap();
        let _ = r.healthy();
    }

    #[tokio::test]
    async fn repair_unknown_issue_returns_unsupported() {
        let ops = DefaultNativeOps::new();
        let err = ops.repair(&["unknown-issue".into()], ProgressSink::noop()).await.unwrap_err();
        assert!(matches!(err, OpsError::Unsupported { .. }));
    }

    #[tokio::test]
    async fn reinstall_unknown_component_returns_notfound() {
        let ops = DefaultNativeOps::new();
        let err = ops.reinstall_component("unknown", ProgressSink::noop()).await.unwrap_err();
        assert!(matches!(err, OpsError::NotFound { .. }));
    }
}
