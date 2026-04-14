use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{SandboxBackend, SandboxOpts, ResourceStats, InstallMode, ImageSource};

pub struct LimaBackend {
    vm_name: String,
}

impl LimaBackend {
    pub fn new(instance_name: &str) -> Self {
        Self {
            vm_name: format!("clawenv-{instance_name}"),
        }
    }

    /// Create with an explicit VM name (used when sandbox_id already contains full name)
    pub fn new_with_vm_name(vm_name: &str) -> Self {
        Self { vm_name: vm_name.to_string() }
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

/// Parse two /proc/stat "cpu" lines into a CPU usage percentage.
fn parse_lima_cpu_usage(line1: &str, line2: &str) -> f32 {
    fn parse_fields(line: &str) -> Option<(u64, u64)> {
        let parts: Vec<u64> = line.split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() < 4 {
            return None;
        }
        let idle = parts[3] + parts.get(4).unwrap_or(&0);
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
        if self.is_available().await? {
            return Ok(());
        }

        tracing::info!("Lima not found, attempting to install...");

        // Strategy 1: Try Homebrew (if available)
        let has_brew = Command::new("which")
            .arg("brew")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if has_brew {
            tracing::info!("Homebrew found, installing Lima via brew...");
            let status = Command::new("brew")
                .args(["install", "lima"])
                .status()
                .await?;
            if status.success() {
                return Ok(());
            }
            tracing::warn!("Homebrew install failed, trying direct download...");
        }

        // Strategy 2: Direct binary download from GitHub releases
        tracing::info!("Downloading Lima binary directly from GitHub...");
        let arch = match std::env::consts::ARCH {
            "aarch64" => "aarch64",
            "x86_64" => "x86_64",
            other => anyhow::bail!("Unsupported architecture for Lima: {other}"),
        };

        let url = format!(
            "https://github.com/lima-vm/lima/releases/latest/download/lima-{arch}-apple-darwin.tar.gz"
        );

        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let install_dir = format!("{home}/.local");
        tokio::fs::create_dir_all(&install_dir).await?;

        // Download and extract
        let status = Command::new("sh")
            .args(["-c", &format!(
                "curl -fsSL '{url}' | tar xz -C '{install_dir}'"
            )])
            .status()
            .await?;

        if !status.success() {
            anyhow::bail!(
                "Failed to install Lima. Please install manually:\n\
                 - macOS: brew install lima\n\
                 - Or download from: https://github.com/lima-vm/lima/releases"
            );
        }

        // Add to PATH hint
        let bin_path = format!("{install_dir}/bin");
        if !std::env::var("PATH").unwrap_or_default().contains(&bin_path) {
            tracing::info!("Lima installed to {bin_path}. Add to PATH: export PATH=\"{bin_path}:$PATH\"");
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
                let template = include_str!("../../../assets/lima/clawenv-alpine.yaml");

                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                let workspace_dir = format!("{}/.clawenv/workspaces/{}", home, opts.instance_name);
                let gateway_port = opts.gateway_port;
                let ttyd_port = crate::manager::install::allocate_port(gateway_port, 1);
                let bridge_port = crate::manager::install::allocate_port(gateway_port, 2);
                let cdp_port = crate::manager::install::allocate_port(gateway_port, 3);
                let vnc_ws_port = crate::manager::install::allocate_port(gateway_port, 4);

                let mut rendered = template
                    .replace("{WORKSPACE_DIR}", &workspace_dir)
                    .replace("{GATEWAY_PORT}", &gateway_port.to_string())
                    .replace("{TTYD_PORT}", &ttyd_port.to_string())
                    .replace("{BRIDGE_PORT}", &bridge_port.to_string())
                    .replace("{CDP_PORT}", &cdp_port.to_string())
                    .replace("{VNC_WS_PORT}", &vnc_ws_port.to_string())
                    .replace("{PROXY_SCRIPT}", &opts.proxy_script)
                    .replace("{MIRRORS_SCRIPT}", "");

                // Replace Alpine image download URLs if a custom mirror is configured
                if !opts.alpine_mirror.is_empty() {
                    rendered = rendered.replace(
                        "https://dl-cdn.alpinelinux.org/alpine",
                        &opts.alpine_mirror,
                    );
                }

                // Write rendered template
                let templates_dir = Self::templates_dir()?;
                tokio::fs::create_dir_all(&templates_dir).await?;
                tokio::fs::create_dir_all(&workspace_dir).await?;
                let template_path = templates_dir.join(format!("{}.yaml", self.vm_name));
                tokio::fs::write(&template_path, &rendered).await?;

                tracing::info!("Creating Lima VM '{}' with provision (packages + OpenClaw)", self.vm_name);

                // limactl start blocks until provision completes (~7-10 min)
                self.limactl_run(
                    &["start", "--name", &self.vm_name, "--tty=false",
                      &template_path.to_string_lossy()],
                ).await?;

                tracing::info!("Lima VM '{}' created and provisioned", self.vm_name);
            }
            _ => anyhow::bail!("Install mode not supported by Lima backend"),
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
        // Plan C: spawn with pipes, join!(wait, read, read) with timeout
        let args = ["shell", &self.vm_name, "--", "sh", "-c", cmd];
        let (stdout, stderr, rc) = super::exec_helper::exec("limactl", &args).await?;
        if rc != 0 {
            anyhow::bail!("exec failed (exit {rc}): {cmd}\nstdout: {}\nstderr: {}",
                stdout.chars().take(500).collect::<String>(),
                stderr.chars().take(500).collect::<String>());
        }
        Ok(stdout)
    }

    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String> {
        let args = ["shell", &self.vm_name, "--", "sh", "-c", cmd];
        let (output, rc) = super::exec_helper::exec_with_progress("limactl", &args, tx).await?;
        if rc != 0 {
            anyhow::bail!("command failed (exit {rc}): {cmd}");
        }
        Ok(output)
    }

    async fn stats(&self) -> Result<ResourceStats> {
        // Query Lima VM config for memory limit
        let output = self.limactl(&["list", "--json"]).await?;

        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct LimaVm {
            name: String,
            #[serde(default)]
            cpus: u32,
            #[serde(default)]
            memory: u64,
        }

        let vms: Vec<LimaVm> = serde_json::from_str(&output).unwrap_or_default();
        let memory_limit_mb = vms.iter()
            .find(|v| v.name == self.vm_name)
            .map(|vm| vm.memory / (1024 * 1024))
            .unwrap_or(0);

        if memory_limit_mb == 0 {
            return Ok(ResourceStats::default());
        }

        // Query real memory usage from inside the VM via /proc/meminfo
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
        let memory_used_mb = if mem_total_kb > 0 {
            (mem_total_kb / 1024).saturating_sub(mem_available_kb / 1024)
        } else {
            0
        };

        // Query CPU usage from /proc/stat (two samples, 1s apart)
        let cpu_percent = match self.exec(
            "head -1 /proc/stat; sleep 1; head -1 /proc/stat"
        ).await {
            Ok(output) => {
                let lines: Vec<&str> = output.lines().collect();
                if lines.len() >= 2 {
                    parse_lima_cpu_usage(lines[0], lines[1])
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

        let lima_base = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?
            .join(".lima");
        let vm_dir = lima_base.join(&self.vm_name);

        if vm_dir.exists() {
            anyhow::bail!("Lima instance '{}' already exists at {}", self.vm_name, vm_dir.display());
        }

        // Extract to a temp directory first to avoid conflicts with existing VMs
        let tmp_dir = lima_base.join(format!("_import_tmp_{}", std::process::id()));
        tokio::fs::create_dir_all(&tmp_dir).await?;

        let status = tokio::process::Command::new("tar")
            .args(["xzf", &path.to_string_lossy(), "-C", &tmp_dir.to_string_lossy()])
            .status()
            .await?;
        if !status.success() {
            tokio::fs::remove_dir_all(&tmp_dir).await.ok();
            anyhow::bail!("Failed to extract Lima image from {}", path.display());
        }

        // Find the extracted directory (should contain lima.yaml)
        let mut extracted_path = None;
        let mut entries = tokio::fs::read_dir(&tmp_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().join("lima.yaml").exists() {
                extracted_path = Some(entry.path());
                break;
            }
        }

        let src = match extracted_path {
            Some(p) => p,
            None => {
                tokio::fs::remove_dir_all(&tmp_dir).await.ok();
                anyhow::bail!("Extracted archive does not contain a Lima VM (no lima.yaml found)");
            }
        };

        // Move to final location with target vm_name
        tokio::fs::rename(&src, &vm_dir).await?;
        tokio::fs::remove_dir_all(&tmp_dir).await.ok();

        // Start the imported VM
        self.limactl(&["start", &self.vm_name]).await?;
        Ok(())
    }

    async fn rename(&self, new_name: &str) -> Result<String> {
        let new_vm = format!("clawenv-{new_name}");
        self.limactl_run(&["rename", &self.vm_name, &new_vm]).await?;
        Ok(new_vm)
    }

    async fn edit_resources(&self, cpus: Option<u32>, memory_mb: Option<u32>, disk_gb: Option<u32>) -> Result<()> {
        let mut args = vec!["edit".to_string(), self.vm_name.clone()];
        if let Some(c) = cpus {
            args.push("--cpus".into());
            args.push(c.to_string());
        }
        if let Some(m) = memory_mb {
            args.push("--memory".into());
            // Lima uses GiB float
            args.push(format!("{:.1}", m as f64 / 1024.0));
        }
        if let Some(d) = disk_gb {
            args.push("--disk".into());
            args.push(d.to_string());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.limactl_run(&arg_refs).await
    }

    async fn edit_port_forwards(&self, forwards: &[(u16, u16)]) -> Result<()> {
        // Build yq expression for portForwards array
        let entries: Vec<String> = forwards.iter()
            .map(|(guest, host)| format!("{{\"guestPort\":{guest},\"hostPort\":{host}}}"))
            .collect();
        let yq_expr = format!(".portForwards = [{}]", entries.join(","));
        self.limactl_run(&["edit", &self.vm_name, "--set", &yq_expr]).await
    }

    fn supports_rename(&self) -> bool { true }
    fn supports_resource_edit(&self) -> bool { true }
    fn supports_port_edit(&self) -> bool { true }
}
