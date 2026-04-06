use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{SandboxBackend, SandboxOpts, SnapshotInfo, ResourceStats, InstallMode, ImageSource};

pub struct LimaBackend {
    vm_name: String,
}

impl LimaBackend {
    pub fn new(instance_name: &str) -> Self {
        Self {
            vm_name: format!("clawenv-{instance_name}"),
        }
    }

    /// Run limactl and capture stdout (for commands that exit quickly like list, shell)
    async fn limactl(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("limactl")
            .args(args)
            .output()
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("limactl {} failed: {}", args.join(" "), stderr);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Run limactl without capturing output (for long-running commands like start)
    /// Lima's hostagent inherits pipes and keeps them open, so .output() would hang.
    async fn limactl_run(&self, args: &[&str]) -> Result<()> {
        let status = Command::new("limactl")
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("limactl {} failed (exit code {:?})", args.join(" "), status.code());
        }
        Ok(())
    }

    /// Run limactl with streaming stderr output (for long operations like start)
    async fn limactl_stream(&self, args: &[&str], tx: Option<&mpsc::Sender<String>>) -> Result<String> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = Command::new("limactl")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stderr = child.stderr.take();
        let stdout = child.stdout.take();

        // Stream stderr (Lima outputs progress here)
        if let (Some(stderr), Some(tx)) = (stderr, tx) {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx.send(line).await;
                }
            });
        }

        let status = child.wait().await?;
        let stdout_data = if let Some(mut so) = stdout {
            let mut buf = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut so, &mut buf).await?;
            String::from_utf8_lossy(&buf).to_string()
        } else {
            String::new()
        };

        if !status.success() {
            anyhow::bail!("limactl {} failed (exit code {:?})", args.join(" "), status.code());
        }
        Ok(stdout_data)
    }

    fn templates_dir() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/templates"))
    }

    /// Download a remote image with checksum verification
    async fn download_image(url: &str, checksum_sha256: &str) -> Result<PathBuf> {
        use std::io::Write;

        let cache_dir = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/cache");
        tokio::fs::create_dir_all(&cache_dir).await?;

        let filename = url.rsplit('/').next().unwrap_or("image.qcow2");
        let dest = cache_dir.join(filename);

        // Skip download if already cached with correct checksum
        if dest.exists() {
            let existing = tokio::fs::read(&dest).await?;
            let hash = sha256_hex(&existing);
            if hash == checksum_sha256 {
                tracing::info!("Using cached image: {}", dest.display());
                return Ok(dest);
            }
        }

        tracing::info!("Downloading image from {url}...");
        let resp = reqwest::get(url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("Download failed: HTTP {}", resp.status());
        }
        let bytes = resp.bytes().await?;

        // Verify checksum
        let hash = sha256_hex(&bytes);
        if hash != checksum_sha256 {
            anyhow::bail!(
                "Checksum mismatch: expected {checksum_sha256}, got {hash}"
            );
        }

        let mut file = std::fs::File::create(&dest)?;
        file.write_all(&bytes)?;
        tracing::info!("Image downloaded to {}", dest.display());
        Ok(dest)
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(data);
    hex::encode(hash)
}

#[async_trait]
impl SandboxBackend for LimaBackend {
    fn name(&self) -> &str {
        "Lima + Alpine Linux"
    }

    async fn is_available(&self) -> Result<bool> {
        let result = Command::new("limactl")
            .args(["--version"])
            .output()
            .await;
        Ok(result.map(|o| o.status.success()).unwrap_or(false))
    }

    async fn ensure_prerequisites(&self) -> Result<()> {
        if !self.is_available().await? {
            tracing::info!("Lima not found, installing via Homebrew...");
            let status = Command::new("brew")
                .args(["install", "lima"])
                .status()
                .await?;
            if !status.success() {
                anyhow::bail!("Failed to install Lima via Homebrew");
            }
        }
        Ok(())
    }

    async fn create(&self, opts: &SandboxOpts) -> Result<()> {
        match &opts.install_mode {
            InstallMode::PrebuiltImage { source } => {
                let path = match source {
                    ImageSource::LocalFile { path } => path.clone(),
                    ImageSource::Remote { url, checksum_sha256 } => {
                        Self::download_image(url, checksum_sha256).await?
                    }
                };
                self.import_image(&path).await?;
            }
            InstallMode::OnlineBuild => {
                // Use Lima's built-in Alpine template (has SSH + cloud-init pre-configured)
                // Then provision OpenClaw inside the running VM
                tracing::info!("Creating Lima VM '{}' from template:alpine", self.vm_name);

                self.limactl_run(
                    &["start", "--name", &self.vm_name, "--tty=false", "template:alpine"],
                ).await?;

                tracing::info!("Lima VM '{}' created and running", self.vm_name);

                // Provisioning is done by install.rs (not here) so each step
                // can send individual progress updates to the frontend.

                // Optional: install browser if requested (also moved to install.rs)
                if opts.install_browser {
                    tracing::info!("Browser install will be handled by install flow");
                }
            }
        }
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        self.limactl_run(&["start", &self.vm_name]).await
    }

    async fn stop(&self) -> Result<()> {
        self.limactl(&["stop", &self.vm_name]).await?;
        Ok(())
    }

    async fn destroy(&self) -> Result<()> {
        self.limactl(&["delete", &self.vm_name, "--force"]).await?;
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        // Use a two-phase approach:
        // 1. Spawn with piped stdout, read with a timeout guard
        // 2. If the command writes output we need, capture it
        // For most exec calls, we just need the exit code.
        //
        // Lima's shell can keep pipes open (hostagent inheritance),
        // so we use spawn + read_to_end with concurrent wait.
        use tokio::io::AsyncReadExt;

        let mut child = Command::new("limactl")
            .args(["shell", &self.vm_name, "--", "sh", "-c", cmd])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null()) // don't capture stderr to avoid pipe hang
            .kill_on_drop(true)
            .spawn()?;

        let stdout_handle = child.stdout.take();

        // Wait for process to exit first
        let status = child.wait().await?;

        // Then read whatever stdout is available (process is done, pipe should close)
        let stdout_data = if let Some(mut so) = stdout_handle {
            let mut buf = Vec::new();
            // Use a short timeout for reading — if pipe doesn't close, give up
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                so.read_to_end(&mut buf),
            ).await {
                Ok(Ok(_)) => String::from_utf8_lossy(&buf).to_string(),
                _ => String::new(),
            }
        } else {
            String::new()
        };

        if !status.success() {
            anyhow::bail!("exec failed (exit {:?}): {}", status.code(), cmd);
        }
        Ok(stdout_data)
    }

    async fn exec_stream(&self, cmd: &str, tx: mpsc::Sender<String>) -> Result<ExitStatus> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = Command::new("limactl")
            .args(["shell", &self.vm_name, "--", "sh", "-c", cmd])
            .stdout(std::process::Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
        let mut reader = BufReader::new(stdout).lines();

        while let Some(line) = reader.next_line().await? {
            let _ = tx.send(line).await;
        }

        Ok(child.wait().await?)
    }

    async fn snapshot_create(&self, tag: &str) -> Result<()> {
        self.limactl(&["snapshot", "create", &self.vm_name, "--tag", tag]).await?;
        Ok(())
    }

    async fn snapshot_restore(&self, tag: &str) -> Result<()> {
        self.limactl(&["snapshot", "apply", &self.vm_name, "--tag", tag]).await?;
        Ok(())
    }

    async fn snapshot_list(&self) -> Result<Vec<SnapshotInfo>> {
        let output = self.limactl(&["snapshot", "list", &self.vm_name, "--json"]).await;
        match output {
            Ok(json_str) => {
                // Parse JSON output — limactl returns array of snapshots
                #[derive(serde::Deserialize)]
                struct LimaSnapshot {
                    tag: String,
                    #[serde(default)]
                    created: String,
                }
                let snaps: Vec<LimaSnapshot> = serde_json::from_str(&json_str).unwrap_or_default();
                Ok(snaps.into_iter().map(|s| SnapshotInfo {
                    tag: s.tag,
                    created_at: s.created.parse().unwrap_or_else(|_| chrono::Utc::now()),
                    size_bytes: 0,
                }).collect())
            }
            Err(_) => Ok(vec![]),
        }
    }

    async fn stats(&self) -> Result<ResourceStats> {
        // Query Lima VM info for resource usage
        let output = self.limactl(&["list", "--json"]).await?;

        #[derive(serde::Deserialize)]
        struct LimaVm {
            name: String,
            #[serde(default)]
            cpus: u32,
            #[serde(default)]
            memory: u64,
        }

        let vms: Vec<LimaVm> = serde_json::from_str(&output).unwrap_or_default();
        if let Some(vm) = vms.iter().find(|v| v.name == self.vm_name) {
            Ok(ResourceStats {
                cpu_percent: 0.0, // Lima doesn't report real-time CPU
                memory_used_mb: 0,
                memory_limit_mb: vm.memory / (1024 * 1024),
            })
        } else {
            Ok(ResourceStats::default())
        }
    }

    async fn import_image(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            anyhow::bail!("Image file not found: {}", path.display());
        }
        // For Lima, import as a disk image
        self.limactl(&[
            "create",
            "--name", &self.vm_name,
            &path.to_string_lossy(),
        ]).await?;
        self.limactl(&["start", &self.vm_name]).await?;
        Ok(())
    }
}
