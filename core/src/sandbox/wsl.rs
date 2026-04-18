use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{ImageSource, InstallMode, SandboxBackend, SandboxOpts, ResourceStats};

pub struct WslBackend {
    distro_name: String,
}

impl WslBackend {
    pub fn new(instance_name: &str) -> Self {
        Self {
            distro_name: format!("ClawEnv-{instance_name}"),
        }
    }

    pub fn new_with_distro_name(distro_name: &str) -> Self {
        Self { distro_name: distro_name.to_string() }
    }

    fn distro_dir(&self) -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/wsl")
            .join(&self.distro_name))
    }

    fn cache_dir() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/cache"))
    }

    async fn wsl_cmd(&self, args: &[&str]) -> Result<String> {
        let out = crate::platform::process::silent_cmd("wsl")
            .args(args)
            .output()
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("wsl {} failed: {}", args.join(" "), stderr);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Detect system architecture for Alpine downloads
    fn alpine_arch() -> &'static str {
        match std::env::consts::ARCH {
            "aarch64" => "aarch64",
            _ => "x86_64",
        }
    }

    /// Query the latest Alpine version from the CDN.
    /// Parses the directory listing of /alpine/latest-stable/releases/{arch}/
    async fn resolve_latest_alpine_version() -> Result<String> {
        let arch = Self::alpine_arch();
        let url = format!(
            "https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/{arch}/"
        );
        tracing::info!("Querying latest Alpine version from {url}...");

        let resp = reqwest::get(&url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to query Alpine releases: HTTP {}", resp.status());
        }
        let body = resp.text().await?;

        // Parse directory listing for "alpine-minirootfs-X.Y.Z-{arch}.tar.gz"
        let prefix = "alpine-minirootfs-".to_string();
        let suffix = format!("-{arch}.tar.gz");
        let mut versions: Vec<String> = Vec::new();
        for segment in body.split('"') {
            if segment.starts_with(&prefix) && segment.ends_with(&suffix) {
                let ver = &segment[prefix.len()..segment.len() - suffix.len()];
                if ver.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    versions.push(ver.to_string());
                }
            }
        }

        versions.sort_by(|a, b| {
            let parse = |s: &str| -> Vec<u32> {
                s.split('.').filter_map(|p| p.parse().ok()).collect()
            };
            parse(b).cmp(&parse(a))
        });

        versions.into_iter().next()
            .ok_or_else(|| anyhow::anyhow!("Could not find Alpine minirootfs for {arch} at {url}"))
    }

    async fn download_alpine_rootfs(alpine_version: &str) -> Result<PathBuf> {
        use std::io::Write;

        let cache_dir = Self::cache_dir()?;
        tokio::fs::create_dir_all(&cache_dir).await?;

        // Step 1: resolve actual version if "latest-stable"
        let version = if alpine_version == "latest-stable" || alpine_version.contains("latest") {
            Self::resolve_latest_alpine_version().await?
        } else {
            alpine_version.to_string()
        };

        // Step 2: detect architecture
        let arch = Self::alpine_arch();
        let filename = format!("alpine-minirootfs-{version}-{arch}.tar.gz");
        let dest = cache_dir.join(&filename);

        if dest.exists() {
            tracing::info!("Using cached Alpine rootfs: {}", dest.display());
            return Ok(dest);
        }

        // Extract major.minor for the URL path (e.g., "3.23.2" → "3.23")
        let parts: Vec<&str> = version.split('.').collect();
        let major_minor = if parts.len() >= 2 {
            format!("{}.{}", parts[0], parts[1])
        } else {
            version.clone()
        };

        let url = format!(
            "https://dl-cdn.alpinelinux.org/alpine/v{major_minor}/releases/{arch}/{filename}"
        );

        tracing::info!("Downloading Alpine rootfs from {url}...");
        let resp = reqwest::get(&url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("Download failed: HTTP {} for {url}", resp.status());
        }
        let bytes = resp.bytes().await?;

        let mut file = std::fs::File::create(&dest)?;
        file.write_all(&bytes)?;
        tracing::info!("Alpine rootfs {} downloaded to {}", filename, dest.display());
        Ok(dest)
    }
}

#[async_trait]
impl SandboxBackend for WslBackend {
    fn name(&self) -> &str {
        "WSL2 + Alpine Linux"
    }

    async fn is_available(&self) -> Result<bool> {
        use crate::platform::process::silent_cmd;

        // Check both required features using dism (no window flash)
        let wsl_check = silent_cmd("dism")
            .args(["/online", "/get-featureinfo", "/featurename:Microsoft-Windows-Subsystem-Linux", "/English"])
            .output().await;
        let wsl_ok = wsl_check.as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("Enabled"))
            .unwrap_or(false);

        if !wsl_ok {
            return Ok(false);
        }

        let vm_check = silent_cmd("dism")
            .args(["/online", "/get-featureinfo", "/featurename:VirtualMachinePlatform", "/English"])
            .output().await;
        let vm_ok = vm_check.as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("Enabled"))
            .unwrap_or(false);

        if !vm_ok {
            return Ok(false);
        }

        // Both features enabled — verify wsl.exe responds
        let wsl_status = silent_cmd("wsl")
            .args(["--status"])
            .output().await;
        Ok(wsl_status.map(|o| o.status.success()).unwrap_or(false))
    }

    async fn ensure_prerequisites(&self) -> Result<()> {
        use crate::platform::process::silent_cmd;

        // First check: is hardware virtualization available?
        // WSL2 requires Hyper-V which needs hardware virtualization support.
        // Fails inside VMs without nested virtualization (e.g., UTM on Apple Silicon).
        let hyperv_check = silent_cmd("dism")
            .args(["/online", "/get-featureinfo", "/featurename:Microsoft-Hyper-V", "/English"])
            .output().await;
        let _hyperv_available = hyperv_check.as_ref()
            .map(|o| {
                let s = String::from_utf8_lossy(&o.stdout);
                // If the feature exists (even if Disabled), hardware can support it
                s.contains("State :") && !s.contains("not recognized")
            })
            .unwrap_or(false);

        // Also check if we're in a VM that doesn't support nested virtualization
        let vm_check = silent_cmd("powershell")
            .args(["-WindowStyle", "Hidden", "-Command",
                "(Get-WmiObject Win32_ComputerSystem).Model"])
            .output().await;
        let is_virtual = vm_check.as_ref()
            .map(|o| {
                let model = String::from_utf8_lossy(&o.stdout).to_lowercase();
                model.contains("virtual") || model.contains("vmware") || model.contains("qemu")
                    || model.contains("utm") || model.contains("parallels")
            })
            .unwrap_or(false);

        if is_virtual {
            // Check if nested virtualization actually works
            let nested_check = silent_cmd("powershell")
                .args(["-WindowStyle", "Hidden", "-Command",
                    "(Get-CimInstance Win32_Processor).VirtualizationFirmwareEnabled"])
                .output().await;
            let nested_ok = nested_check.as_ref()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().contains("True"))
                .unwrap_or(false);

            if !nested_ok {
                anyhow::bail!(
                    "WSL2 requires hardware virtualization, which is not available in this virtual machine.\n\
                     \n\
                     This computer appears to be a virtual machine without nested virtualization support.\n\
                     WSL2 cannot run inside VMs that don't support nested virtualization.\n\
                     \n\
                     Options:\n\
                     - Use a physical Windows PC for sandbox installation\n\
                     - Use 'Native' install mode instead (no sandbox, runs directly on host)\n\
                     - Use a cloud VM with nested virtualization (e.g., Azure Dv5 series)"
                );
            }
        }

        if self.is_available().await? {
            return Ok(());
        }

        // Check what's missing using dism (no window flash, works reliably)
        let wsl_check = silent_cmd("dism")
            .args(["/online", "/get-featureinfo", "/featurename:Microsoft-Windows-Subsystem-Linux", "/English"])
            .output().await;
        let wsl_enabled = wsl_check.as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("Enabled"))
            .unwrap_or(false);

        let vm_check = silent_cmd("dism")
            .args(["/online", "/get-featureinfo", "/featurename:VirtualMachinePlatform", "/English"])
            .output().await;
        let vm_enabled = vm_check.as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("Enabled"))
            .unwrap_or(false);

        tracing::info!("WSL feature: {}, VM platform: {}", wsl_enabled, vm_enabled);

        if wsl_enabled && vm_enabled {
            // Both features enabled — try setting WSL2 as default
            let _ = silent_cmd("wsl")
                .args(["--set-default-version", "2"])
                .output().await;
            // Also try updating the WSL kernel
            let _ = silent_cmd("wsl")
                .args(["--update"])
                .output().await;
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if self.is_available().await? {
                return Ok(());
            }
        }

        // Need to enable features — use dism via UAC elevation (shows terminal, intentional)
        tracing::info!("Installing WSL2 (UAC elevation required)...");

        // Build a PowerShell script that enables both features explicitly
        let install_script = r#"
            $ErrorActionPreference = 'Stop'
            Write-Host 'Enabling Windows Subsystem for Linux...'
            dism /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart
            Write-Host 'Enabling Virtual Machine Platform...'
            dism /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart
            Write-Host 'Installing WSL kernel update...'
            wsl --update 2>$null
            Write-Host 'Done. A restart may be required.'
            Start-Sleep -Seconds 3
        "#;

        // This is the ONLY command that intentionally shows a terminal window
        let status = Command::new("powershell")
            .args(["-Command", &format!(
                "Start-Process -FilePath 'powershell' -ArgumentList '-ExecutionPolicy Bypass -Command {}' -Verb RunAs -Wait",
                install_script.replace('\n', "; ").replace('\'', "''")
            )])
            .status()
            .await;

        if status.map(|s| s.success()).unwrap_or(false) {
            // Check if it worked without restart
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if self.is_available().await? {
                return Ok(());
            }
            // Needs restart
            anyhow::bail!(
                "WSL2 installation completed. A system restart is required.\n\
                 \n\
                 Click 'Restart Now' to restart your computer,\n\
                 then open ClawEnv again after restart."
            );
        } else {
            anyhow::bail!(
                "WSL2 installation was cancelled or failed.\n\
                 \n\
                 To install manually:\n\
                 1. Open PowerShell as Administrator\n\
                 2. Run: dism /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart\n\
                 3. Run: dism /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart\n\
                 4. Restart your computer\n\
                 5. Open ClawEnv again"
            );
        }
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
                // Resolve the claw descriptor to get the correct npm package name
                let claw_reg = crate::claw::ClawRegistry::load();
                let desc = claw_reg.get(&opts.claw_type);
                let npm_install = desc.npm_install_cmd(&opts.claw_version);
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

echo "STAGE:claw" >> "$LOG"
{npm_install} >> "$LOG" 2>&1

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

                // Run in background and capture PID for tracking
                self.exec("nohup sh /tmp/clawenv-provision.sh > /dev/null 2>&1 & echo $! > /tmp/clawenv-provision.pid").await?;

                // Poll for completion using both PID tracking (new) and done-file (legacy compatible)
                let mut elapsed = 0u64;
                let mut last_lines = 0usize;
                let mut idle = 0u64;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    elapsed += 5;

                    // Check done-file first (works with both old and new scripts)
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

                    // Done-file written — script completed (success or failure)
                    if !done.trim().is_empty() {
                        self.exec("rm -f /tmp/clawenv-provision.sh /tmp/clawenv-provision.log /tmp/clawenv-provision.done /tmp/clawenv-provision.pid").await.ok();
                        let rc: i32 = done.trim().parse().unwrap_or(-1);
                        if rc != 0 {
                            anyhow::bail!("WSL provision failed (exit {rc})");
                        }
                        break;
                    }

                    // PID-based crash detection: if process exited but done-file was never written,
                    // the script crashed. This catches the case that previously hung for 10 minutes.
                    if idle >= 30 {
                        let pid_check = self.exec(
                            "PID=$(cat /tmp/clawenv-provision.pid 2>/dev/null); \
                             if [ -n \"$PID\" ] && ! kill -0 $PID 2>/dev/null; then echo 'dead'; else echo 'alive'; fi"
                        ).await.unwrap_or_default();
                        if pid_check.trim() == "dead" {
                            // Process crashed without writing done-file — extract last error from log
                            let tail = self.exec("tail -5 /tmp/clawenv-provision.log 2>/dev/null || echo 'no log'").await.unwrap_or_default();
                            self.exec("rm -f /tmp/clawenv-provision.sh /tmp/clawenv-provision.log /tmp/clawenv-provision.done /tmp/clawenv-provision.pid").await.ok();
                            anyhow::bail!(
                                "WSL provision script crashed (process exited without completion).\n\
                                 Last log output:\n{tail}"
                            );
                        }
                    }

                    if idle >= 600 {
                        // Kill the stalled process before bailing
                        self.exec("PID=$(cat /tmp/clawenv-provision.pid 2>/dev/null); [ -n \"$PID\" ] && kill $PID 2>/dev/null").await.ok();
                        self.exec("rm -f /tmp/clawenv-provision.sh /tmp/clawenv-provision.log /tmp/clawenv-provision.done /tmp/clawenv-provision.pid").await.ok();
                        anyhow::bail!("WSL provision stalled — no output for 10 min");
                    }
                }
            }
            _ => anyhow::bail!("Install mode not supported by WSL2 backend"),
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

    async fn stats(&self) -> Result<ResourceStats> {
        // Verify the distro exists
        let list_output = self.wsl_cmd(&["--list", "--verbose"]).await?;
        if !list_output.lines().any(|l| l.contains(&self.distro_name)) {
            return Ok(ResourceStats::default());
        }

        // Query memory from /proc/meminfo inside the WSL distro
        let meminfo = self.exec("cat /proc/meminfo 2>/dev/null || echo ''").await.unwrap_or_default();
        let mut mem_total_kb: u64 = 0;
        let mut mem_available_kb: u64 = 0;
        for line in meminfo.lines() {
            if let Some(val) = line.strip_prefix("MemTotal:") {
                mem_total_kb = val.trim().strip_suffix("kB").unwrap_or(val.trim())
                    .trim().parse().unwrap_or(0);
            } else if let Some(val) = line.strip_prefix("MemAvailable:") {
                mem_available_kb = val.trim().strip_suffix("kB").unwrap_or(val.trim())
                    .trim().parse().unwrap_or(0);
            }
        }
        let memory_limit_mb = mem_total_kb / 1024;
        let memory_used_mb = memory_limit_mb.saturating_sub(mem_available_kb / 1024);

        // Query CPU usage from /proc/stat (two samples, 1s apart)
        let cpu_percent = match self.exec(
            "head -1 /proc/stat; sleep 1; head -1 /proc/stat"
        ).await {
            Ok(output) => {
                let lines: Vec<&str> = output.lines().collect();
                if lines.len() >= 2 {
                    parse_cpu_usage(lines[0], lines[1])
                } else {
                    0.0
                }
            }
            Err(_) => 0.0,
        };

        Ok(ResourceStats {
            cpu_percent,
            memory_used_mb,
            memory_limit_mb,
        })
    }

    async fn import_image(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            anyhow::bail!("Image file not found: {}", path.display());
        }

        // v0.2.6+ bundles are wrapped: outer tar.gz with `clawenv-bundle.toml`
        // + `payload.tar` (the raw `wsl --export`). Peek manifest to
        // validate, then unwrap payload into a temp file for `wsl --import`.
        let manifest = crate::export::BundleManifest::peek_from_tarball(path).await
            .map_err(|e| anyhow!(
                "Cannot import WSL bundle from {}: {e}",
                path.display()
            ))?;
        if manifest.sandbox_type != crate::sandbox::SandboxType::Wsl2Alpine.as_wire_str() {
            anyhow::bail!(
                "Bundle {} declares sandbox_type '{}' but this is the WSL importer.",
                path.display(), manifest.sandbox_type
            );
        }

        let tmp = std::env::temp_dir().join(format!(
            "clawenv-wsl-import-{}.tar", std::process::id()
        ));
        crate::export::BundleManifest::extract_inner_payload(path, &tmp).await?;

        let distro_dir = self.distro_dir()?;
        tokio::fs::create_dir_all(&distro_dir).await?;
        let distro_path = distro_dir.to_string_lossy().to_string();
        let import_result = self.wsl_cmd(&[
            "--import", &self.distro_name, &distro_path,
            &tmp.to_string_lossy(), "--version", "2",
        ]).await;
        let _ = tokio::fs::remove_file(&tmp).await;
        import_result?;

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

    async fn edit_port_forwards(&self, forwards: &[(u16, u16)]) -> Result<()> {
        // WSL2 port forwarding via netsh interface portproxy
        // First, clear existing ClawEnv port proxies by removing known ports
        // Then add new ones

        // Get the WSL2 distro's IP address
        let wsl_ip = self.exec("hostname -I 2>/dev/null | awk '{print $1}'")
            .await?
            .trim()
            .to_string();
        if wsl_ip.is_empty() {
            anyhow::bail!("Cannot determine WSL2 distro IP address");
        }

        // Remove all existing portproxy rules, then add new ones
        // Reset v4tov4 portproxy (requires admin, but so does adding rules)
        for &(guest_port, host_port) in forwards {
            // Delete existing rule (ignore errors if it doesn't exist)
            let _ = Command::new("netsh")
                .args(["interface", "portproxy", "delete", "v4tov4",
                       &format!("listenport={host_port}"), "listenaddress=127.0.0.1"])
                .output().await;

            // Add new rule
            let out = Command::new("netsh")
                .args(["interface", "portproxy", "add", "v4tov4",
                       &format!("listenport={host_port}"), "listenaddress=127.0.0.1",
                       &format!("connectport={guest_port}"), &format!("connectaddress={wsl_ip}")])
                .output().await?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                if stderr.contains("denied") || stderr.contains("1314") {
                    anyhow::bail!(
                        "WSL2 port forwarding requires Administrator privileges.\n\
                         Please run ClawEnv as Administrator.\n\
                         netsh error: {stderr}"
                    );
                }
                anyhow::bail!("netsh portproxy add failed for {host_port}->{guest_port}: {stderr}");
            }
        }

        tracing::info!("WSL2 port forwards updated: {:?} (via {})", forwards, wsl_ip);
        Ok(())
    }

    fn supports_rename(&self) -> bool { true }
    fn supports_resource_edit(&self) -> bool { true }
    fn supports_port_edit(&self) -> bool { true }
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

/// Parse two /proc/stat "cpu" lines into a CPU usage percentage.
/// Format: cpu user nice system idle iowait irq softirq steal
fn parse_cpu_usage(line1: &str, line2: &str) -> f32 {
    fn parse_fields(line: &str) -> Option<(u64, u64)> {
        let parts: Vec<u64> = line.split_whitespace()
            .skip(1) // skip "cpu"
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() < 4 {
            return None;
        }
        let idle = parts[3] + parts.get(4).unwrap_or(&0); // idle + iowait
        let total: u64 = parts.iter().sum();
        Some((idle, total))
    }

    let (Some((idle1, total1)), Some((idle2, total2))) =
        (parse_fields(line1), parse_fields(line2)) else { return 0.0 };

    let total_diff = total2.saturating_sub(total1);
    let idle_diff = idle2.saturating_sub(idle1);
    if total_diff == 0 {
        return 0.0;
    }
    ((total_diff - idle_diff) as f32 / total_diff as f32) * 100.0
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(data);
    hex::encode(hash)
}
