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
        // Native install lives at `<clawenv_root>/native` — honours the
        // `CLAWENV_HOME` env var for test isolation, same as every other
        // path in the codebase. Must match install_native/mod.rs's
        // `install_dir` computation exactly; any drift means npm's cwd
        // ends up in a different tree from where install_native actually
        // dropped node_modules.
        let install_dir = crate::config::clawenv_root().join("native");
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
        // Mirror Lima/Podman/Wsl behaviour: surface stderr in the error on
        // non-zero exit so callers see the actual failure reason (previously
        // only the stdout was returned and exit code was ignored, making
        // native-mode install failures opaque).
        let out = self.shell.cmd(cmd)
            .current_dir(&self.install_dir)
            .output()
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            anyhow::bail!(
                "command failed (exit {:?}): {}\nstdout: {}\nstderr: {}",
                out.status.code(), cmd,
                stdout.chars().take(500).collect::<String>(),
                stderr.chars().take(500).collect::<String>()
            );
        }
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
        // Collect stderr tail in-memory so we can include it in the error
        // message on non-zero exit. tx still receives every stderr line live
        // (streams to the UI as before).
        let stderr_buf: std::sync::Arc<tokio::sync::Mutex<String>> =
            std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));

        if let Some(out) = stdout {
            let mut reader = BufReader::new(out).lines();
            let stderr_tx = tx.clone();
            let stderr_buf_c = stderr_buf.clone();
            let stderr_task = stderr.map(|err| tokio::spawn(async move {
                    let mut reader = BufReader::new(err).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        {
                            let mut buf = stderr_buf_c.lock().await;
                            buf.push_str(&line);
                            buf.push('\n');
                        }
                        let _ = stderr_tx.send(line).await;
                    }
                }));

            while let Ok(Some(line)) = reader.next_line().await {
                output.push_str(&line);
                output.push('\n');
                let _ = tx.send(line).await;
            }

            if let Some(t) = stderr_task { t.await.ok(); }
        }

        let status = child.wait().await?;
        if !status.success() {
            let err_tail = {
                let b = stderr_buf.lock().await;
                b.chars().rev().take(800).collect::<String>().chars().rev().collect::<String>()
            };
            anyhow::bail!(
                "command failed (exit {:?}): {}\nstderr tail:\n{}",
                status.code(), cmd, err_tail
            );
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
    // Native has no VM port-forward layer — config.toml carries the port
    // number and the gateway binary binds it directly. There's nothing for
    // edit_port_forwards to do, so returning true misled the UI into
    // showing a port-edit action that silently no-oped.
    fn supports_port_edit(&self) -> bool { false }
}
