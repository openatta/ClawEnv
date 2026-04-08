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

    fn distro_dir(&self) -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/wsl")
            .join(&self.distro_name))
    }

    fn snapshot_dir(&self) -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/wsl/snapshots")
            .join(&self.distro_name))
    }

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
        if self.is_available().await? {
            return Ok(());
        }

        let wsl_check = Command::new("wsl")
            .args(["--status"])
            .output().await;

        let msg = match wsl_check {
            Ok(out) if out.status.success() => {
                "WSL is installed but may not be configured for WSL2.\n\
                 Please run in PowerShell (Administrator):\n\
                   wsl --set-default-version 2"
                    .to_string()
            }
            _ => {
                // WSL2 not installed — need system restart after install
                "WSL2 is not installed on your system.\n\
                 \n\
                 To install (requires Administrator + system restart):\n\
                 1. Open PowerShell as Administrator\n\
                 2. Run: wsl --install\n\
                 3. Restart your computer\n\
                 4. Run ClawEnv again\n\
                 \n\
                 Note: A system restart is required after WSL2 installation.\n\
                 ClawEnv cannot proceed until WSL2 is available.\n\
                 \n\
                 See https://learn.microsoft.com/en-us/windows/wsl/install"
                    .to_string()
            }
        };

        anyhow::bail!("{msg}");
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
                            let hash = sha256_hex(&bytes);
                            if hash != *checksum_sha256 {
                                anyhow::bail!("Checksum mismatch: expected {checksum_sha256}, got {hash}");
                            }
                            let mut file = std::fs::File::create(&dest)?;
                            file.write_all(&bytes)?;
                        }
                        dest
                    }
                };
                self.wsl_cmd(&[
                    "--import", &self.distro_name, &distro_path,
                    &rootfs_path.to_string_lossy(), "--version", "2",
                ]).await?;
            }
            InstallMode::OnlineBuild => {
                // Download Alpine minirootfs and import
                let rootfs = Self::download_alpine_rootfs(&opts.alpine_version).await?;
                self.wsl_cmd(&[
                    "--import", &self.distro_name, &distro_path,
                    &rootfs.to_string_lossy(), "--version", "2",
                ]).await?;

                // Run provision as a single script inside WSL (avoids pipe timeout)
                // Write script → run with nohup → poll done file
                let proxy = &opts.proxy_script;
                let version = &opts.claw_version;
                let browser_cmd = if opts.install_browser {
                    "apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont"
                } else {
                    "echo 'browser skipped'"
                };

                let provision_script = format!(r#"#!/bin/sh
LOG=/tmp/clawenv-provision.log
DONE=/tmp/clawenv-provision.done
rm -f "$LOG" "$DONE"

echo "STAGE:proxy" > "$LOG"
{proxy}

echo "STAGE:packages" >> "$LOG"
apk update >> "$LOG" 2>&1
apk add --no-cache git curl bash nodejs npm ttyd openssh build-base python3 >> "$LOG" 2>&1

echo "STAGE:user" >> "$LOG"
adduser -D -s /bin/bash clawenv >> "$LOG" 2>&1 || true
echo "clawenv ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers

echo "STAGE:ssh" >> "$LOG"
ssh-keygen -A >> "$LOG" 2>&1

echo "STAGE:openclaw" >> "$LOG"
npm install -g --loglevel http openclaw@{version} >> "$LOG" 2>&1

echo "STAGE:browser" >> "$LOG"
{browser_cmd} >> "$LOG" 2>&1

echo "STAGE:done" >> "$LOG"
echo "0" > "$DONE"
"#);

                // Write provision script into WSL
                self.exec(&format!(
                    "cat > /tmp/clawenv-provision.sh << 'PROVEOF'\n{provision_script}\nPROVEOF"
                )).await?;
                self.exec("chmod +x /tmp/clawenv-provision.sh").await?;

                // Run in background (decoupled from pipe)
                self.exec("nohup sh /tmp/clawenv-provision.sh > /dev/null 2>&1 &").await?;

                // Poll for completion
                let mut elapsed = 0u64;
                let mut last_lines = 0usize;
                let mut idle = 0u64;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    elapsed += 5;

                    let done = self.exec("cat /tmp/clawenv-provision.done 2>/dev/null || echo ''").await.unwrap_or_default();
                    let log = self.exec(&format!(
                        "tail -n +{} /tmp/clawenv-provision.log 2>/dev/null | head -30 || echo ''",
                        last_lines + 1
                    )).await.unwrap_or_default();

                    let new_lines: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
                    if !new_lines.is_empty() {
                        idle = 0;
                        last_lines += new_lines.len();
                        tracing::info!("[WSL provision {elapsed}s] {}", new_lines.last().unwrap_or(&""));
                    } else {
                        idle += 5;
                    }

                    if !done.trim().is_empty() {
                        self.exec("rm -f /tmp/clawenv-provision.sh /tmp/clawenv-provision.log /tmp/clawenv-provision.done").await.ok();
                        let rc: i32 = done.trim().parse().unwrap_or(-1);
                        if rc != 0 {
                            anyhow::bail!("WSL provision failed (exit {rc})");
                        }
                        break;
                    }

                    if idle >= 600 {
                        anyhow::bail!("WSL provision stalled — no output for 10 min");
                    }
                }
            }
        }
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        self.wsl_cmd(&["-d", &self.distro_name, "--", "echo", "started"]).await?;
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.wsl_cmd(&["--terminate", &self.distro_name]).await?;
        Ok(())
    }

    async fn destroy(&self) -> Result<()> {
        self.wsl_cmd(&["--unregister", &self.distro_name]).await?;
        let distro_dir = self.distro_dir()?;
        if distro_dir.exists() {
            tokio::fs::remove_dir_all(&distro_dir).await?;
        }
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
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
        self.wsl_cmd(&["--export", &self.distro_name, &snapshot_path.to_string_lossy()]).await?;
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
        self.wsl_cmd(&["--unregister", &self.distro_name]).await?;
        self.wsl_cmd(&[
            "--import", &self.distro_name, &distro_path,
            &snapshot_path.to_string_lossy(), "--version", "2",
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
                let tag = path.file_stem().and_then(|s| s.to_str())
                    .unwrap_or("").trim_end_matches(".tar").to_string();
                let metadata = entry.metadata().await?;
                let created = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
                let created_at: chrono::DateTime<chrono::Utc> = created.into();
                snapshots.push(SnapshotInfo { tag, created_at, size_bytes: metadata.len() });
            }
        }

        Ok(snapshots)
    }

    async fn stats(&self) -> Result<ResourceStats> {
        let output = self.wsl_cmd(&["--list", "--verbose"]).await?;
        for line in output.lines() {
            if line.contains(&self.distro_name) {
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
            "--import", &self.distro_name, &distro_path,
            &path.to_string_lossy(), "--version", "2",
        ]).await?;
        tracing::info!("Image imported as WSL distro '{}'", self.distro_name);
        Ok(())
    }

    // ---- Management operations ----

    async fn rename(&self, new_name: &str) -> Result<String> {
        // WSL has no rename command — export → unregister → import with new name
        let snap_dir = Self::cache_dir()?;
        tokio::fs::create_dir_all(&snap_dir).await?;
        let tmp_export = snap_dir.join("_rename_tmp.tar.gz");

        // Export current distro
        self.wsl_cmd(&["--export", &self.distro_name, &tmp_export.to_string_lossy()]).await?;

        // Unregister old
        self.wsl_cmd(&["--unregister", &self.distro_name]).await?;
        let old_dir = self.distro_dir()?;
        if old_dir.exists() {
            tokio::fs::remove_dir_all(&old_dir).await?;
        }

        // Import with new name
        let new_distro = format!("ClawEnv-{new_name}");
        let new_dir = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/wsl")
            .join(&new_distro);
        tokio::fs::create_dir_all(&new_dir).await?;

        let result = Command::new("wsl")
            .args(["--import", &new_distro, &new_dir.to_string_lossy(), &tmp_export.to_string_lossy(), "--version", "2"])
            .output().await?;

        // Cleanup temp file
        tokio::fs::remove_file(&tmp_export).await.ok();

        if !result.status.success() {
            anyhow::bail!("Failed to import renamed distro: {}", String::from_utf8_lossy(&result.stderr));
        }

        Ok(new_distro)
    }

    async fn edit_resources(&self, cpus: Option<u32>, memory_mb: Option<u32>, _disk_gb: Option<u32>) -> Result<()> {
        // WSL2 resources are configured via %USERPROFILE%\.wslconfig (global for all distros)
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".into());
        let wslconfig_path = PathBuf::from(&home).join(".wslconfig");

        let mut config = if wslconfig_path.exists() {
            tokio::fs::read_to_string(&wslconfig_path).await.unwrap_or_default()
        } else {
            String::new()
        };

        // Ensure [wsl2] section exists
        if !config.contains("[wsl2]") {
            config.push_str("\n[wsl2]\n");
        }

        // Update or add settings
        if let Some(c) = cpus {
            config = set_wslconfig_value(&config, "processors", &c.to_string());
        }
        if let Some(m) = memory_mb {
            let gb = format!("{}GB", m / 1024);
            config = set_wslconfig_value(&config, "memory", &gb);
        }

        tokio::fs::write(&wslconfig_path, &config).await?;
        tracing::info!("Updated .wslconfig: cpus={:?}, memory={:?}", cpus, memory_mb);
        Ok(())
    }

    fn supports_rename(&self) -> bool { true }
    fn supports_resource_edit(&self) -> bool { true }
    // WSL2 auto-forwards localhost ports, no portForwards needed
    fn supports_port_edit(&self) -> bool { false }
}

/// Update or insert a key=value in .wslconfig under [wsl2] section
fn set_wslconfig_value(config: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = config.lines().map(|l| l.to_string()).collect();
    let key_lower = key.to_lowercase();

    // Find existing key
    let mut found = false;
    for line in lines.iter_mut() {
        let trimmed = line.trim().to_lowercase();
        if trimmed.starts_with(&key_lower) && trimmed.contains('=') {
            *line = format!("{}={}", key, value);
            found = true;
            break;
        }
    }

    if !found {
        // Insert after [wsl2] line
        let insert_pos = lines.iter().position(|l| l.trim() == "[wsl2]")
            .map(|i| i + 1)
            .unwrap_or(lines.len());
        lines.insert(insert_pos, format!("{}={}", key, value));
    }

    lines.join("\n")
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(data);
    hex::encode(hash)
}
