//! Native mode — runs directly on host, no sandbox isolation.
//!
//! All command execution goes through ManagedShell to ensure
//! ClawEnv's own Node.js and Git are used, never system ones.

use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::{SandboxBackend, SandboxOpts, ResourceStats};
use crate::platform::managed_shell::ManagedShell;

pub struct NativeBackend {
    shell: ManagedShell,
    install_dir: PathBuf,
}

impl NativeBackend {
    pub fn new(_dir_hint: &str) -> Self {
        let shell = ManagedShell::new();
        let install_dir = shell.inst_bin_dir().parent()
            .unwrap_or(&shell.inst_bin_dir())
            .to_path_buf();
        // On Windows inst_bin_dir() == install_dir, on Unix inst_bin_dir() == install_dir/bin
        let install_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".clawenv").join("native");
        Self { shell, install_dir }
    }

    /// Access the managed shell (for external callers like instance.rs)
    pub fn shell(&self) -> &ManagedShell {
        &self.shell
    }
}

#[async_trait]
impl SandboxBackend for NativeBackend {
    fn name(&self) -> &str {
        "Native (no sandbox)"
    }

    async fn is_available(&self) -> Result<bool> {
        let check = self.shell.cmd("node --version")
            .output().await
            .map(|o| o.status.success()).unwrap_or(false);
        Ok(check)
    }

    async fn ensure_prerequisites(&self) -> Result<()> {
        if !self.is_available().await? {
            anyhow::bail!("Native mode requires Node.js. Please run the installer first.");
        }
        Ok(())
    }

    async fn create(&self, opts: &SandboxOpts) -> Result<()> {
        tokio::fs::create_dir_all(&self.install_dir).await?;
        let registry = crate::claw::ClawRegistry::load();
        let desc = registry.get(&opts.claw_type);
        let install_cmd = desc.npm_install_cmd(&opts.claw_version);
        let status = self.shell.cmd(&install_cmd)
            .current_dir(&self.install_dir)
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("Failed to install {}", desc.display_name);
        }
        Ok(())
    }

    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self) -> Result<()> { Ok(()) }

    async fn destroy(&self) -> Result<()> {
        if self.install_dir.exists() {
            tokio::fs::remove_dir_all(&self.install_dir).await?;
        }
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        let out = self.shell.cmd(cmd)
            .current_dir(&self.install_dir)
            .output()
            .await?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = self.shell.cmd(cmd)
            .current_dir(&self.install_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let mut output = String::new();

        if let Some(out) = stdout {
            let mut reader = BufReader::new(out).lines();
            let stderr_tx = tx.clone();
            let stderr_task = if let Some(err) = stderr {
                Some(tokio::spawn(async move {
                    let mut reader = BufReader::new(err).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        let _ = stderr_tx.send(line).await;
                    }
                }))
            } else { None };

            while let Ok(Some(line)) = reader.next_line().await {
                output.push_str(&line);
                output.push('\n');
                let _ = tx.send(line).await;
            }

            if let Some(t) = stderr_task { t.await.ok(); }
        }

        let status = child.wait().await?;
        if !status.success() {
            anyhow::bail!("command failed (exit {:?}): {}", status.code(), cmd);
        }
        Ok(output)
    }

    async fn edit_resources(&self, _cpus: Option<u32>, _memory_mb: Option<u32>, _disk_gb: Option<u32>) -> Result<()> {
        Ok(()) // Native has no resource limits to edit
    }

    async fn edit_port_forwards(&self, _forwards: &[(u16, u16)]) -> Result<()> {
        Ok(()) // Native doesn't need port forwarding
    }

    async fn stats(&self) -> Result<ResourceStats> {
        Ok(ResourceStats::default())
    }

    async fn import_image(&self, _path: &std::path::Path) -> Result<()> {
        anyhow::bail!("Native mode does not support image import")
    }

    fn supports_rename(&self) -> bool { false }
    fn supports_resource_edit(&self) -> bool { false }
    fn supports_port_edit(&self) -> bool { true }
}
