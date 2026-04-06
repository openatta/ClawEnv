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

    async fn limactl(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("limactl")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("limactl {} failed: {}", args.join(" "), stderr);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
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
                let template = include_str!("../../../assets/lima/clawenv-alpine.yaml");

                // Detect architecture
                let arch = match std::env::consts::ARCH {
                    "aarch64" => "aarch64",
                    "x86_64" => "x86_64",
                    other => anyhow::bail!("Unsupported architecture: {other}"),
                };

                let rendered = template
                    .replace("{INSTANCE_NAME}", &opts.instance_name)
                    .replace("{OPENCLAW_VERSION}", &opts.claw_version)
                    .replace("{ARCH}", arch);

                // Ensure workspace directory exists
                let workspace_dir = dirs::home_dir()
                    .ok_or_else(|| anyhow!("Cannot find home directory"))?
                    .join(".clawenv/workspaces")
                    .join(&opts.instance_name);
                tokio::fs::create_dir_all(&workspace_dir).await?;

                let templates_dir = Self::templates_dir()?;
                tokio::fs::create_dir_all(&templates_dir).await?;
                let template_path = templates_dir.join(format!("{}.yaml", self.vm_name));
                tokio::fs::write(&template_path, &rendered).await?;

                self.limactl(&[
                    "start",
                    "--name", &self.vm_name,
                    "--tty=false",
                    &template_path.to_string_lossy(),
                ]).await?;

                // Optional: install browser if requested
                if opts.install_browser {
                    self.exec(
                        "apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont"
                    ).await?;
                }
            }
        }
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        self.limactl(&["start", &self.vm_name]).await?;
        Ok(())
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
        let out = Command::new("limactl")
            .args(["shell", &self.vm_name, "--", "ash", "-c", cmd])
            .output()
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("exec in sandbox failed: {stderr}");
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn exec_stream(&self, cmd: &str, tx: mpsc::Sender<String>) -> Result<ExitStatus> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = Command::new("limactl")
            .args(["shell", &self.vm_name, "--", "ash", "-c", cmd])
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
