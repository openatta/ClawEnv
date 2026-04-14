//! Native 模式——直接在宿主机运行，无沙盒隔离（开发者专用）
//!
//! Platform-specific shell execution:
//!   macOS/Linux: sh -c "..."
//!   Windows:     powershell -WindowStyle Hidden -Command "..."

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{SandboxBackend, SandboxOpts, ResourceStats};

pub struct NativeBackend {
    install_dir: PathBuf,
}

impl NativeBackend {
    /// Create backend. `_dir_hint` is ignored — native always uses ~/.clawenv/native/.
    pub fn new(_dir_hint: &str) -> Self {
        let install_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".clawenv").join("native");
        Self { install_dir }
    }

    /// Build PATH with ClawEnv's own Node.js, Git, and instance bin dir prepended.
    fn clawenv_path(&self) -> String {
        let home = dirs::home_dir().unwrap_or_default();
        let clawenv = home.join(".clawenv");

        #[cfg(target_os = "windows")]
        let node_bin = clawenv.join("node");
        #[cfg(not(target_os = "windows"))]
        let node_bin = clawenv.join("node").join("bin");

        #[cfg(target_os = "windows")]
        let git_bin = clawenv.join("git").join("cmd");
        #[cfg(not(target_os = "windows"))]
        let git_bin = clawenv.join("git").join("bin");

        #[cfg(target_os = "windows")]
        let inst_bin = self.install_dir.clone();
        #[cfg(not(target_os = "windows"))]
        let inst_bin = self.install_dir.join("bin");

        let mut current = std::env::var("PATH").unwrap_or_default();

        // Also include system Git paths as fallback (Windows)
        #[cfg(target_os = "windows")]
        {
            for p in [r"C:\Program Files\Git\cmd", r"C:\Program Files\LLVM\bin"] {
                if std::path::Path::new(p).exists() && !current.contains(p) {
                    current = format!("{};{}", current, p);
                }
            }
        }

        #[cfg(target_os = "windows")]
        { format!("{};{};{};{}", node_bin.display(), git_bin.display(), inst_bin.display(), current) }
        #[cfg(not(target_os = "windows"))]
        { format!("{}:{}:{}:{}", node_bin.display(), git_bin.display(), inst_bin.display(), current) }
    }

    /// Create a platform-appropriate shell command with ClawEnv node in PATH.
    pub fn shell_cmd_with_path(&self, cmd: &str) -> Command {
        let path = self.clawenv_path();
        #[cfg(target_os = "windows")]
        {
            let mut c = crate::platform::process::silent_cmd("powershell");
            // Inject PATH before running the command
            let full = format!("$env:PATH = '{}'; {}", path.replace('\'', "''"), cmd);
            c.args(["-Command", &full]);
            c
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut c = Command::new("sh");
            c.args(["-c", &format!("export PATH='{}'; {}", path, cmd)]);
            c
        }
    }

    /// Legacy static method for compatibility (does NOT inject PATH).
    pub fn shell_cmd(cmd: &str) -> Command {
        #[cfg(target_os = "windows")]
        {
            let mut c = crate::platform::process::silent_cmd("powershell");
            c.args(["-Command", cmd]);
            c
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut c = Command::new("sh");
            c.args(["-c", cmd]);
            c
        }
    }
}

#[async_trait]
impl SandboxBackend for NativeBackend {
    fn name(&self) -> &str {
        "Native (no sandbox)"
    }

    async fn is_available(&self) -> Result<bool> {
        use crate::platform::process::silent_cmd;
        let node = silent_cmd("node").args(["--version"]).output().await;
        let npm = silent_cmd("npm").args(["--version"]).output().await;
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
        let registry = crate::claw::ClawRegistry::load();
        let desc = registry.get(&opts.claw_type);
        let install_cmd = desc.npm_install_cmd(&opts.claw_version);
        let status = Self::shell_cmd(&install_cmd)
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
        let out = self.shell_cmd_with_path(cmd)
            .current_dir(&self.install_dir)
            .output()
            .await?;
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = self.shell_cmd_with_path(cmd)
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
