use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{ImageSource, InstallMode, SandboxBackend, SandboxOpts, ResourceStats};

pub struct PodmanBackend {
    container_name: String,
    image_tag: String,
    port: u16,
}

impl PodmanBackend {
    pub fn new(instance_name: &str, version: &str) -> Self {
        Self {
            container_name: format!("clawenv-{instance_name}"),
            image_tag: format!("clawenv/openclaw:{version}"),
            port: 3000,
        }
    }

    /// Create with default version tag
    pub fn with_defaults(instance_name: &str) -> Self {
        Self::new(instance_name, "latest")
    }

    /// Create with specific port (for multi-instance)
    pub fn with_port(instance_name: &str, port: u16) -> Self {
        let mut b = Self::new(instance_name, "latest");
        b.port = port;
        b
    }

    /// Run podman command without capturing output (for long commands like build/start)
    async fn podman_run(&self, args: &[&str]) -> Result<()> {
        let status = Command::new("podman")
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("podman {} failed (exit {:?})", args.join(" "), status.code());
        }
        Ok(())
    }

    /// Run podman command, capturing stdout. For short commands.
    async fn podman(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("podman")
            .args(args)
            .output()
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("podman {} failed: {}", args.join(" "), stderr);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Path to the bundled Containerfile
    fn containerfile_path() -> Result<PathBuf> {
        // Look relative to the project assets directory
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = PathBuf::from(manifest_dir)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("assets/podman/Containerfile");
        if path.exists() {
            Ok(path)
        } else {
            // Fallback: check in home dir cache
            Ok(dirs::home_dir()
                .ok_or_else(|| anyhow!("Cannot find home directory"))?
                .join(".clawenv/assets/Containerfile"))
        }
    }

    /// Workspace directory for bind-mounting into the container
    fn workspace_dir(instance_name: &str) -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv/workspaces")
            .join(instance_name))
    }
}

#[async_trait]
impl SandboxBackend for PodmanBackend {
    fn name(&self) -> &str {
        "Podman + Alpine Linux"
    }

    async fn is_available(&self) -> Result<bool> {
        let result = Command::new("podman")
            .args(["--version"])
            .output()
            .await;
        Ok(result.map(|o| o.status.success()).unwrap_or(false))
    }

    async fn ensure_prerequisites(&self) -> Result<()> {
        if self.is_available().await? {
            return Ok(());
        }

        tracing::info!("Podman not found, attempting to install...");

        // Detect package manager and try auto-install
        let pkg_managers = [
            ("apt-get", &["install", "-y", "podman"][..]),
            ("dnf", &["install", "-y", "podman"][..]),
            ("pacman", &["-S", "--noconfirm", "podman"][..]),
            ("zypper", &["install", "-y", "podman"][..]),
        ];

        for (pm, args) in &pkg_managers {
            let has_pm = Command::new("which")
                .arg(pm)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status().await
                .map(|s| s.success()).unwrap_or(false);

            if has_pm {
                tracing::info!("Found {pm}, installing podman...");
                let status = Command::new("sudo")
                    .arg(pm)
                    .args(*args)
                    .status().await?;
                if status.success() && self.is_available().await? {
                    return Ok(());
                }
                tracing::warn!("{pm} install failed, trying next...");
            }
        }

        anyhow::bail!(
            "Could not install Podman automatically.\n\
             Please install manually:\n\
             - Fedora/RHEL: sudo dnf install podman\n\
             - Ubuntu/Debian: sudo apt install podman\n\
             - Arch: sudo pacman -S podman\n\
             See https://podman.io/docs/installation"
        );
    }

    async fn create(&self, opts: &SandboxOpts) -> Result<()> {
        match &opts.install_mode {
            InstallMode::PrebuiltImage { source } => {
                let image_path = match source {
                    ImageSource::LocalFile { path } => path.clone(),
                    ImageSource::Remote { url, checksum_sha256 } => {
                        use std::io::Write;
                        let cache_dir = dirs::home_dir()
                            .ok_or_else(|| anyhow!("Cannot find home directory"))?
                            .join(".clawenv/cache");
                        tokio::fs::create_dir_all(&cache_dir).await?;

                        let filename = url.rsplit('/').next().unwrap_or("image.tar");
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
                self.podman(&["load", "-i", &image_path.to_string_lossy()]).await?;
            }
            InstallMode::OnlineBuild => {
                let containerfile = Self::containerfile_path()?;
                if !containerfile.exists() {
                    anyhow::bail!(
                        "Containerfile not found at {}. Ensure the assets/podman/Containerfile exists.",
                        containerfile.display()
                    );
                }

                let context_dir = containerfile.parent()
                    .ok_or_else(|| anyhow!("Invalid Containerfile path"))?;

                let install_browser = if opts.install_browser { "true" } else { "false" };

                self.podman_run(&[
                    "build",
                    "-t", &self.image_tag,
                    "--build-arg", &format!("OPENCLAW_VERSION={}", opts.claw_version),
                    "--build-arg", &format!("INSTALL_BROWSER={}", install_browser),
                    "-f", &containerfile.to_string_lossy(),
                    &context_dir.to_string_lossy(),
                ]).await?;
            }
        }

        tracing::info!("Podman image '{}' ready", self.image_tag);
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        // Extract instance name from container name (strip "clawenv-" prefix)
        let instance_name = self.container_name.strip_prefix("clawenv-").unwrap_or(&self.container_name);
        let workspace = Self::workspace_dir(instance_name)?;
        tokio::fs::create_dir_all(&workspace).await?;

        self.podman_run(&[
            "run", "-d",
            "--name", &self.container_name,
            "--userns=keep-id",
            "-v", &format!("{}:/workspace:Z", workspace.to_string_lossy()),
            "-p", &format!("127.0.0.1:{}:3000", self.port),
            &self.image_tag,
        ]).await?;

        tracing::info!("Container '{}' started", self.container_name);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.podman(&["stop", &self.container_name]).await?;
        Ok(())
    }

    async fn destroy(&self) -> Result<()> {
        // Force remove container
        self.podman(&["rm", "-f", &self.container_name]).await.ok();
        // Remove image
        self.podman(&["rmi", &self.image_tag]).await.ok();
        tracing::info!("Container '{}' and image '{}' removed", self.container_name, self.image_tag);
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        // Plan C: spawn with pipes, join!(wait, read, read) with timeout
        let args = ["exec", &self.container_name.as_str(), "sh", "-c", cmd];
        let (stdout, stderr, rc) = super::exec_helper::exec("podman", &args).await?;
        if rc != 0 {
            anyhow::bail!("exec in Podman failed (exit {rc}): {cmd}\nstdout: {}\nstderr: {}",
                stdout.chars().take(500).collect::<String>(),
                stderr.chars().take(500).collect::<String>());
        }
        Ok(stdout)
    }

    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String> {
        let args = ["exec", &self.container_name.as_str(), "sh", "-c", cmd];
        let (output, rc) = super::exec_helper::exec_with_progress("podman", &args, tx).await?;
        if rc != 0 {
            anyhow::bail!("command failed in Podman (exit {rc}): {cmd}");
        }
        Ok(output)
    }

    async fn stats(&self) -> Result<ResourceStats> {
        let output = self.podman(&[
            "stats", "--no-stream", "--format", "json", &self.container_name,
        ]).await?;

        // Podman stats JSON is an array of objects
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct PodmanStats {
            #[serde(alias = "CPU", alias = "cpu_percent", default)]
            cpu: String,
            #[serde(alias = "MemUsage", alias = "mem_usage", default)]
            mem_usage: String,
            #[serde(alias = "MemLimit", alias = "mem_limit", default)]
            mem_limit: String,
        }

        let stats_list: Vec<PodmanStats> = serde_json::from_str(&output).unwrap_or_default();

        if let Some(s) = stats_list.first() {
            let cpu_percent = s.cpu.trim_end_matches('%').parse::<f32>().unwrap_or(0.0);

            // Parse memory values (e.g., "128MiB / 8GiB")
            let mem_parts: Vec<&str> = s.mem_usage.split('/').collect();
            let memory_used_mb = parse_mem_to_mb(mem_parts.first().unwrap_or(&"0"));
            let memory_limit_mb = parse_mem_to_mb(mem_parts.get(1).unwrap_or(&"0"));

            Ok(ResourceStats {
                cpu_percent,
                memory_used_mb,
                memory_limit_mb,
            })
        } else {
            Ok(ResourceStats::default())
        }
    }

    async fn import_image(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            anyhow::bail!("Image file not found: {}", path.display());
        }
        self.podman(&["load", "-i", &path.to_string_lossy()]).await?;
        tracing::info!("Image loaded from {}", path.display());
        Ok(())
    }

    async fn rename(&self, new_name: &str) -> Result<String> {
        let new_container = format!("clawenv-{new_name}");
        self.podman(&["rename", &self.container_name, &new_container]).await?;
        Ok(new_container)
    }

    async fn edit_resources(&self, cpus: Option<u32>, memory_mb: Option<u32>, _disk_gb: Option<u32>) -> Result<()> {
        let mut args = vec!["update".to_string(), self.container_name.clone()];
        if let Some(c) = cpus {
            args.push("--cpus".into());
            args.push(c.to_string());
        }
        if let Some(m) = memory_mb {
            args.push("--memory".into());
            args.push(format!("{m}m"));
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        self.podman(&arg_refs).await?;
        Ok(())
    }

    async fn edit_port_forwards(&self, forwards: &[(u16, u16)]) -> Result<()> {
        // Podman port binding is set at container creation — must recreate
        // 1. Commit current container state to a temporary image
        // 2. Stop & remove old container
        // 3. Run new container with updated ports from the committed image

        let tmp_image = format!("{}:port-edit-tmp", self.image_tag);

        // Commit current state
        self.podman(&["commit", &self.container_name, &tmp_image]).await?;

        // Stop and remove old container
        self.podman(&["stop", &self.container_name]).await.ok();
        self.podman(&["rm", "-f", &self.container_name]).await?;

        // Build port args
        let instance_name = self.container_name.strip_prefix("clawenv-").unwrap_or(&self.container_name);
        let workspace = Self::workspace_dir(instance_name)?;

        let mut run_args = vec![
            "run".to_string(), "-d".to_string(),
            "--name".to_string(), self.container_name.clone(),
            "--userns=keep-id".to_string(),
            "-v".to_string(), format!("{}:/workspace:Z", workspace.to_string_lossy()),
        ];
        for &(guest_port, host_port) in forwards {
            run_args.push("-p".to_string());
            run_args.push(format!("127.0.0.1:{host_port}:{guest_port}"));
        }
        run_args.push(tmp_image.clone());

        let arg_refs: Vec<&str> = run_args.iter().map(|s| s.as_str()).collect();
        let result = self.podman_run(&arg_refs).await;

        // Clean up temp image regardless of success
        self.podman(&["rmi", &tmp_image]).await.ok();

        result?;
        tracing::info!("Podman container '{}' recreated with ports: {:?}", self.container_name, forwards);
        Ok(())
    }

    fn supports_rename(&self) -> bool { true }
    fn supports_resource_edit(&self) -> bool { true }
    fn supports_port_edit(&self) -> bool { true }
}

/// Parse a memory string like "128MiB", "2GiB", "512MB" into megabytes
fn parse_mem_to_mb(s: &str) -> u64 {
    let s = s.trim();
    if let Some(val) = s.strip_suffix("GiB").or_else(|| s.strip_suffix("GB")) {
        val.trim().parse::<f64>().unwrap_or(0.0) as u64 * 1024
    } else if let Some(val) = s.strip_suffix("MiB").or_else(|| s.strip_suffix("MB")) {
        val.trim().parse::<f64>().unwrap_or(0.0) as u64
    } else if let Some(val) = s.strip_suffix("KiB").or_else(|| s.strip_suffix("KB")) {
        val.trim().parse::<f64>().unwrap_or(0.0) as u64 / 1024
    } else {
        0
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let hash = Sha256::digest(data);
    hex::encode(hash)
}
