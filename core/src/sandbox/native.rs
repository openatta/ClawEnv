use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{SandboxBackend, SandboxOpts, ResourceStats};

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

    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&self.install_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let tx2 = tx.clone();
        let stderr_task = tokio::spawn(async move {
            if let Some(se) = stderr {
                let mut reader = BufReader::new(se).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx2.send(line).await;
                }
            }
        });

        let tx3 = tx.clone();
        let stdout_task = tokio::spawn(async move {
            let mut output = String::new();
            if let Some(so) = stdout {
                let mut reader = BufReader::new(so).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx3.send(line.clone()).await;
                    output.push_str(&line);
                    output.push('\n');
                }
            }
            output
        });

        let status = child.wait().await?;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        stderr_task.abort();
        let output = stdout_task.await.unwrap_or_default();

        if !status.success() {
            anyhow::bail!("command failed (exit {:?}): {}", status.code(), cmd);
        }
        Ok(output)
    }

    async fn stats(&self) -> Result<ResourceStats> {
        Ok(ResourceStats::default())
    }

    async fn import_image(&self, _path: &Path) -> Result<()> {
        anyhow::bail!("Native mode does not support image import")
    }
}
