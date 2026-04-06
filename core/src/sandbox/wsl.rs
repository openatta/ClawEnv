use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{ImageSource, InstallMode, SandboxBackend, SandboxOpts, SnapshotInfo, ResourceStats};

pub struct WslBackend {
    distro_name: String,
}

impl WslBackend {
    pub fn new(instance_name: &str) -> Self {
        Self {
            distro_name: format!("ClawEnv-{instance_name}"),
        }
    }

    /// Base directory for WSL distro storage
    fn distro_dir(&self) -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/wsl")
            .join(&self.distro_name))
    }

    /// Directory for snapshot storage
    fn snapshot_dir(&self) -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/wsl/snapshots")
            .join(&self.distro_name))
    }

    /// Cache directory for downloaded rootfs images
    fn cache_dir() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/cache"))
    }

    async fn wsl_cmd(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("wsl")
            .args(args)
            .output()
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("wsl {} failed: {}", args.join(" "), stderr);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Download Alpine minirootfs for WSL import
    async fn download_alpine_rootfs(alpine_version: &str) -> Result<PathBuf> {
        use std::io::Write;

        let cache_dir = Self::cache_dir()?;
        tokio::fs::create_dir_all(&cache_dir).await?;

        let filename = format!("alpine-minirootfs-{}-x86_64.tar.gz", alpine_version);
        let dest = cache_dir.join(&filename);

        if dest.exists() {
            tracing::info!("Using cached Alpine rootfs: {}", dest.display());
            return Ok(dest);
        }

        let url = format!(
            "https://dl-cdn.alpinelinux.org/alpine/v{}/releases/x86_64/{}",
            &alpine_version[..alpine_version.rfind('.').unwrap_or(alpine_version.len())],
            filename
        );

        tracing::info!("Downloading Alpine rootfs from {url}...");
        let resp = reqwest::get(&url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("Download failed: HTTP {}", resp.status());
        }
        let bytes = resp.bytes().await?;

        let mut file = std::fs::File::create(&dest)?;
        file.write_all(&bytes)?;
        tracing::info!("Alpine rootfs downloaded to {}", dest.display());
        Ok(dest)
    }
}

#[async_trait]
impl SandboxBackend for WslBackend {
    fn name(&self) -> &str {
        "WSL2 + Alpine Linux"
    }

    async fn is_available(&self) -> Result<bool> {
        let result = Command::new("wsl")
            .args(["--status"])
            .output()
            .await;
        Ok(result.map(|o| o.status.success()).unwrap_or(false))
    }

    async fn ensure_prerequisites(&self) -> Result<()> {
        if !self.is_available().await? {
            anyhow::bail!(
                "WSL2 is not available. Please enable WSL2 on your system:\n\
                 1. Open PowerShell as Administrator\n\
                 2. Run: wsl --install\n\
                 3. Restart your computer\n\
                 See https://learn.microsoft.com/en-us/windows/wsl/install for details."
            );
        }
        Ok(())
    }

    async fn create(&self, opts: &SandboxOpts) -> Result<()> {
        let distro_dir = self.distro_dir()?;
        tokio::fs::create_dir_all(&distro_dir).await?;
        let distro_path = distro_dir.to_string_lossy().to_string();

        match &opts.install_mode {
            InstallMode::PrebuiltImage { source } => {
                let rootfs_path = match source {
                    ImageSource::LocalFile { path } => path.clone(),
                    ImageSource::Remote { url, checksum_sha256 } => {
                        // Download and verify the image
                        use std::io::Write;
                        let cache_dir = Self::cache_dir()?;
                        tokio::fs::create_dir_all(&cache_dir).await?;

                        let filename = url.rsplit('/').next().unwrap_or("image.tar.gz");
                        let dest = cache_dir.join(filename);

                        if !dest.exists() {
                            tracing::info!("Downloading image from {url}...");
                            let resp = reqwest::get(url).await?;
                            if !resp.status().is_success() {
                                anyhow::bail!("Download failed: HTTP {}", resp.status());
                            }
                            let bytes = resp.bytes().await?;

                            // Verify checksum
                            let hash = sha256_hex(&bytes);
                            if hash != *checksum_sha256 {
                                anyhow::bail!(
                                    "Checksum mismatch: expected {checksum_sha256}, got {hash}"
                                );
                            }

                            let mut file = std::fs::File::create(&dest)?;
                            file.write_all(&bytes)?;
                        }
                        dest
                    }
                };
                self.wsl_cmd(&[
                    "--import",
                    &self.distro_name,
                    &distro_path,
                    &rootfs_path.to_string_lossy(),
                    "--version", "2",
                ]).await?;
            }
            InstallMode::OnlineBuild => {
                // Download Alpine minirootfs
                let rootfs = Self::download_alpine_rootfs(&opts.alpine_version).await?;

                // Import into WSL2
                self.wsl_cmd(&[
                    "--import",
                    &self.distro_name,
                    &distro_path,
                    &rootfs.to_string_lossy(),
                    "--version", "2",
                ]).await?;

                // Install base packages
                self.exec("apk update && apk add --no-cache nodejs npm git curl bash ca-certificates").await?;

                // Install OpenClaw
                self.exec(&format!(
                    "npm install -g openclaw@{} && openclaw --version",
                    opts.claw_version
                )).await?;

                // Optional: install browser packages
                if opts.install_browser {
                    self.exec(
                        "apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont"
                    ).await?;
                }
            }
        }
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        // WSL2 auto-starts on exec; run a no-op to ensure the distro is running
        self.wsl_cmd(&["-d", &self.distro_name, "--", "echo", "started"]).await?;
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.wsl_cmd(&["--terminate", &self.distro_name]).await?;
        Ok(())
    }

    async fn destroy(&self) -> Result<()> {
        self.wsl_cmd(&["--unregister", &self.distro_name]).await?;
        // Clean up local directory
        let distro_dir = self.distro_dir()?;
        if distro_dir.exists() {
            tokio::fs::remove_dir_all(&distro_dir).await?;
        }
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        // Plan C: spawn with pipes, join!(wait, read, read) with timeout
        let args = ["-d", self.distro_name.as_str(), "--", "sh", "-c", cmd];
        let (stdout, stderr, rc) = super::exec_helper::exec("wsl", &args).await?;
        if rc != 0 {
            anyhow::bail!("exec in WSL failed (exit {rc}): {cmd}\nstdout: {}\nstderr: {}",
                stdout.chars().take(500).collect::<String>(),
                stderr.chars().take(500).collect::<String>());
        }
        Ok(stdout)
    }

    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String> {
        let args = ["-d", self.distro_name.as_str(), "--", "sh", "-c", cmd];
        let (output, rc) = super::exec_helper::exec_with_progress("wsl", &args, tx).await?;
        if rc != 0 {
            anyhow::bail!("command failed in WSL (exit {rc}): {cmd}");
        }
        Ok(output)
    }

    async fn snapshot_create(&self, tag: &str) -> Result<()> {
        let snap_dir = self.snapshot_dir()?;
        tokio::fs::create_dir_all(&snap_dir).await?;
        let snapshot_path = snap_dir.join(format!("{tag}.tar.gz"));

        self.wsl_cmd(&[
            "--export",
            &self.distro_name,
            &snapshot_path.to_string_lossy(),
        ]).await?;

        tracing::info!("Snapshot '{}' created at {}", tag, snapshot_path.display());
        Ok(())
    }

    async fn snapshot_restore(&self, tag: &str) -> Result<()> {
        let snap_dir = self.snapshot_dir()?;
        let snapshot_path = snap_dir.join(format!("{tag}.tar.gz"));
        if !snapshot_path.exists() {
            anyhow::bail!("Snapshot '{tag}' not found at {}", snapshot_path.display());
        }

        let distro_dir = self.distro_dir()?;
        let distro_path = distro_dir.to_string_lossy().to_string();

        // Unregister existing distro
        self.wsl_cmd(&["--unregister", &self.distro_name]).await?;

        // Re-import from snapshot
        self.wsl_cmd(&[
            "--import",
            &self.distro_name,
            &distro_path,
            &snapshot_path.to_string_lossy(),
            "--version", "2",
        ]).await?;

        tracing::info!("Snapshot '{}' restored", tag);
        Ok(())
    }

    async fn snapshot_list(&self) -> Result<Vec<SnapshotInfo>> {
        let snap_dir = self.snapshot_dir()?;
        if !snap_dir.exists() {
            return Ok(vec![]);
        }

        let mut entries = tokio::fs::read_dir(&snap_dir).await?;
        let mut snapshots = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("gz") {
                let tag = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .trim_end_matches(".tar")
                    .to_string();
                let metadata = entry.metadata().await?;
                let created = metadata.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                let created_at: chrono::DateTime<chrono::Utc> = created.into();

                snapshots.push(SnapshotInfo {
                    tag,
                    created_at,
                    size_bytes: metadata.len(),
                });
            }
        }

        Ok(snapshots)
    }

    async fn stats(&self) -> Result<ResourceStats> {
        let output = self.wsl_cmd(&["--list", "--verbose"]).await?;

        // Parse `wsl --list --verbose` output for the distro status
        for line in output.lines() {
            if line.contains(&self.distro_name) {
                // WSL doesn't provide detailed resource stats via CLI;
                // return defaults indicating the distro is registered.
                return Ok(ResourceStats {
                    cpu_percent: 0.0,
                    memory_used_mb: 0,
                    memory_limit_mb: 0,
                });
            }
        }

        Ok(ResourceStats::default())
    }

    async fn import_image(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            anyhow::bail!("Image file not found: {}", path.display());
        }

        let distro_dir = self.distro_dir()?;
        tokio::fs::create_dir_all(&distro_dir).await?;
        let distro_path = distro_dir.to_string_lossy().to_string();

        self.wsl_cmd(&[
            "--import",
            &self.distro_name,
            &distro_path,
            &path.to_string_lossy(),
            "--version", "2",
        ]).await?;

        tracing::info!("Image imported as WSL distro '{}'", self.distro_name);
        Ok(())
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(data);
    hex::encode(hash)
}
