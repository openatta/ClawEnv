use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{SandboxBackend, SandboxOpts, SnapshotInfo, ResourceStats};

/// Native 模式——直接在宿主机运行，无沙盒隔离（开发者专用）
pub struct NativeBackend {
    install_dir: PathBuf,
}

impl NativeBackend {
    pub fn new(instance_name: &str) -> Self {
        let install_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".clawenv/native")
            .join(instance_name);
        Self { install_dir }
    }

    fn snapshot_dir(&self) -> PathBuf {
        self.install_dir
            .parent()
            .unwrap_or(&self.install_dir)
            .join("snapshots")
    }
}

#[async_trait]
impl SandboxBackend for NativeBackend {
    fn name(&self) -> &str {
        "Native (no sandbox)"
    }

    async fn is_available(&self) -> Result<bool> {
        let node = Command::new("node").args(["--version"]).output().await;
        let npm = Command::new("npm").args(["--version"]).output().await;
        Ok(node.map(|o| o.status.success()).unwrap_or(false)
            && npm.map(|o| o.status.success()).unwrap_or(false))
    }

    async fn ensure_prerequisites(&self) -> Result<()> {
        if !self.is_available().await? {
            anyhow::bail!("Native mode requires Node.js and npm installed on the host");
        }
        Ok(())
    }

    async fn create(&self, opts: &SandboxOpts) -> Result<()> {
        tokio::fs::create_dir_all(&self.install_dir).await?;
        let status = Command::new("npm")
            .args(["install", "-g", &format!("openclaw@{}", opts.claw_version)])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("Failed to install OpenClaw");
        }
        Ok(())
    }

    // start/stop are no-ops for native mode (no VM/container lifecycle)
    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self) -> Result<()> { Ok(()) }

    async fn destroy(&self) -> Result<()> {
        if self.install_dir.exists() {
            tokio::fs::remove_dir_all(&self.install_dir).await?;
        }
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        let out = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&self.install_dir)
            .output()
            .await?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn exec_stream(&self, cmd: &str, tx: mpsc::Sender<String>) -> Result<ExitStatus> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&self.install_dir)
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
        let snap_dir = self.snapshot_dir();
        tokio::fs::create_dir_all(&snap_dir).await?;
        let snapshot_path = snap_dir.join(format!("{tag}.tar.gz"));
        let status = Command::new("tar")
            .args([
                "-czf", &snapshot_path.to_string_lossy(),
                "-C", &self.install_dir.to_string_lossy(),
                ".",
            ])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("Failed to create snapshot");
        }
        Ok(())
    }

    async fn snapshot_restore(&self, tag: &str) -> Result<()> {
        let snapshot_path = self.snapshot_dir().join(format!("{tag}.tar.gz"));
        if !snapshot_path.exists() {
            anyhow::bail!("Snapshot '{tag}' not found");
        }
        tokio::fs::remove_dir_all(&self.install_dir).await?;
        tokio::fs::create_dir_all(&self.install_dir).await?;
        let status = Command::new("tar")
            .args([
                "-xzf", &snapshot_path.to_string_lossy(),
                "-C", &self.install_dir.to_string_lossy(),
            ])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("Failed to restore snapshot");
        }
        Ok(())
    }

    async fn snapshot_list(&self) -> Result<Vec<SnapshotInfo>> {
        // TODO: list tar.gz files in snapshot dir
        Ok(vec![])
    }

    async fn stats(&self) -> Result<ResourceStats> {
        Ok(ResourceStats::default())
    }

    async fn import_image(&self, _path: &Path) -> Result<()> {
        anyhow::bail!("Native mode does not support image import")
    }
}
