use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{SandboxBackend, SandboxOpts, ResourceStats, InstallMode, ImageSource};

/// Private Lima data directory. VM images, sockets, and state live here
/// so clawenv never touches the system's `~/.lima`.
pub fn lima_home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".clawenv/lima")
}

/// Absolute path to the private `limactl` binary installed by `ensure_prerequisites`.
pub fn limactl_bin() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".clawenv/bin/limactl")
}

/// Initialise Lima env for this process. Must be called once at startup so every
/// spawned `limactl` child inherits `LIMA_HOME` and uses the private data dir.
pub fn init_lima_env() {
    let home = lima_home();
    let _ = std::fs::create_dir_all(&home);
    std::env::set_var("LIMA_HOME", &home);
}

/// Pinned Lima release metadata, loaded from the bundled TOML file.
/// Version bumps are done by editing `assets/lima/lima-release.toml` — no Rust
/// code change required.
struct LimaRelease {
    version: String,
    sha256_arm64: String,
    sha256_x86_64: String,
    url_templates: Vec<String>,
}

impl LimaRelease {
    fn load() -> Result<Self> {
        let toml_src = include_str!("../../../assets/lima/lima-release.toml");
        let table: toml::Table = toml_src.parse()
            .map_err(|e| anyhow!("lima-release.toml is invalid: {e}"))?;

        let version = table.get("version").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("lima-release.toml missing `version`"))?
            .to_string();
        let sha = table.get("sha256").and_then(|v| v.as_table())
            .ok_or_else(|| anyhow!("lima-release.toml missing [sha256] table"))?;
        let sha256_arm64 = sha.get("darwin-arm64").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("lima-release.toml missing sha256.darwin-arm64"))?
            .to_string();
        let sha256_x86_64 = sha.get("darwin-x86_64").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("lima-release.toml missing sha256.darwin-x86_64"))?
            .to_string();

        let url_templates: Vec<String> = table.get("urls")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if url_templates.is_empty() {
            anyhow::bail!("lima-release.toml must list at least one download URL in `urls`");
        }

        Ok(Self { version, sha256_arm64, sha256_x86_64, url_templates })
    }

    /// Render URL templates by substituting {version} and {filename}.
    fn render_urls(&self, filename: &str) -> Vec<String> {
        self.url_templates.iter()
            .map(|t| t.replace("{version}", &self.version).replace("{filename}", filename))
            .collect()
    }
}

/// Try each URL in order, verifying sha256 against the expected value.
/// Returns the first URL whose download matches the checksum. Any HTTP error,
/// body read failure, or checksum mismatch moves on to the next URL.
async fn download_with_fallback(urls: &[String], expected_sha256: &str) -> Result<Vec<u8>> {
    let mut last_err: Option<String> = None;
    for url in urls {
        tracing::info!("Trying {url}");
        match reqwest::get(url).await {
            Err(e) => { last_err = Some(format!("{url}: request failed: {e}")); continue; }
            Ok(resp) if !resp.status().is_success() => {
                last_err = Some(format!("{url}: HTTP {}", resp.status()));
                continue;
            }
            Ok(resp) => {
                match resp.bytes().await {
                    Err(e) => { last_err = Some(format!("{url}: body read failed: {e}")); continue; }
                    Ok(bytes) => {
                        let actual = sha256_hex(&bytes);
                        if actual == expected_sha256 {
                            tracing::info!("Downloaded from {url} ({} bytes, sha256 OK)", bytes.len());
                            return Ok(bytes.to_vec());
                        }
                        last_err = Some(format!(
                            "{url}: checksum mismatch (expected {expected_sha256}, got {actual})"
                        ));
                    }
                }
            }
        }
    }
    anyhow::bail!(
        "All download URLs failed. Last error: {}",
        last_err.as_deref().unwrap_or("(no URLs tried)")
    )
}

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
        let out = Command::new(limactl_bin())
            .args(args)
            .env("LIMA_HOME", lima_home())
            .kill_on_drop(true)
            .output()
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("limactl {} failed: {}", args.join(" "), stderr);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Run limactl without capturing output (for long-running commands like start).
    /// Lima's hostagent inherits pipes and keeps them open, so we can't use
    /// `.output()` or a piped stderr — both would hang. Instead we route stderr
    /// to a temp log file and read the tail only when the command fails, so
    /// users get the real diagnostic message rather than a bare exit code.
    async fn limactl_run(&self, args: &[&str]) -> Result<()> {
        let log_path = std::env::temp_dir().join(format!(
            "clawenv-limactl-{}-{}.log",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let log_file = std::fs::File::create(&log_path)
            .map_err(|e| anyhow!("cannot create limactl log file {}: {e}", log_path.display()))?;

        let status = Command::new(limactl_bin())
            .args(args)
            .env("LIMA_HOME", lima_home())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(log_file))
            .kill_on_drop(true)
            .status()
            .await?;

        if !status.success() {
            let tail = read_log_tail(&log_path, 4000).await;
            let _ = tokio::fs::remove_file(&log_path).await;
            let hint = vm_log_hint(&self.vm_name);
            if tail.trim().is_empty() {
                anyhow::bail!(
                    "limactl {} failed (exit code {:?}){hint}",
                    args.join(" "), status.code()
                );
            } else {
                anyhow::bail!(
                    "limactl {} failed (exit code {:?}):\n{}{hint}",
                    args.join(" "), status.code(), tail.trim()
                );
            }
        }
        let _ = tokio::fs::remove_file(&log_path).await;
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

/// Read the last `max_bytes` of a file as a lossy UTF-8 string. Returns empty
/// on any IO error — the caller decides how to present that.
async fn read_log_tail(path: &Path, max_bytes: usize) -> String {
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(max_bytes);
            String::from_utf8_lossy(&bytes[start..]).to_string()
        }
        Err(_) => String::new(),
    }
}

/// Build a hint pointing at Lima's own hostagent logs under `$LIMA_HOME/<vm>/`
/// when those files exist — those usually contain the root cause when `limactl
/// start` fails mid-provision (cloud-init crash, networking, mount errors).
fn vm_log_hint(vm_name: &str) -> String {
    let vm_dir = lima_home().join(vm_name);
    if !vm_dir.exists() {
        return String::new();
    }
    let candidates = ["ha.stderr.log", "serial.log", "console.log"];
    let existing: Vec<String> = candidates.iter()
        .filter(|f| vm_dir.join(f).exists())
        .map(|f| vm_dir.join(f).display().to_string())
        .collect();
    if existing.is_empty() {
        String::new()
    } else {
        format!("\n\nSee Lima logs for more detail:\n  {}", existing.join("\n  "))
    }
}

/// Locate the VM directory (the one holding `lima.yaml`) inside an extracted
/// tarball, supporting both new (`root/lima/<vm>/`) and old (`root/<vm>/`)
/// export layouts.
async fn find_vm_dir_in_layout(root: &Path) -> Result<Option<PathBuf>> {
    // New layout: look under root/lima/*
    let lima_sub = root.join("lima");
    if lima_sub.is_dir() {
        let mut entries = tokio::fs::read_dir(&lima_sub).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().join("lima.yaml").exists() {
                return Ok(Some(entry.path()));
            }
        }
    }
    // Old layout: any direct child of root containing lima.yaml
    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.path().join("lima.yaml").exists() {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

/// Rewrite absolute host paths in the imported VM's `lima.yaml`. The exporter's
/// workspace path (e.g. `/Users/alice/.clawenv/workspaces/foo`) is replaced
/// with this host's workspace path derived from the target vm_name, and that
/// directory is created so the mount survives first boot.
///
/// The trailing `-<vm_name>` segment is stripped if the vm_name follows the
/// `clawenv-<instance>` convention, so the mount lands under
/// `~/.clawenv/workspaces/<instance>/`.
async fn rewrite_lima_yaml_for_host(vm_dir: &Path, vm_name: &str) -> Result<()> {
    let yaml_path = vm_dir.join("lima.yaml");
    if !yaml_path.exists() {
        return Ok(());
    }
    let content = tokio::fs::read_to_string(&yaml_path).await?;

    let instance_name = vm_name.strip_prefix("clawenv-").unwrap_or(vm_name);
    let home = dirs::home_dir().ok_or_else(|| anyhow!("Cannot find home directory"))?;
    let new_workspace = home.join(".clawenv/workspaces").join(instance_name);
    tokio::fs::create_dir_all(&new_workspace).await?;

    let rewritten: String = content.lines().map(|line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("location:") && line.contains(".clawenv/workspaces") {
            let indent_len = line.len() - trimmed.len();
            let indent = &line[..indent_len];
            format!("{indent}location: \"{}\"", new_workspace.display())
        } else {
            line.to_string()
        }
    }).collect::<Vec<_>>().join("\n");

    tokio::fs::write(&yaml_path, rewritten).await?;
    Ok(())
}

#[async_trait]
impl SandboxBackend for LimaBackend {
    fn name(&self) -> &str {
        "Lima + Alpine Linux"
    }

    async fn is_available(&self) -> Result<bool> {
        let bin = limactl_bin();
        if !bin.exists() {
            return Ok(false);
        }
        let result = Command::new(&bin)
            .args(["--version"])
            .env("LIMA_HOME", lima_home())
            .output()
            .await;
        Ok(result.map(|o| o.status.success()).unwrap_or(false))
    }

    async fn ensure_prerequisites(&self) -> Result<()> {
        if self.is_available().await? {
            return Ok(());
        }

        let release = LimaRelease::load()?;
        let (arch_tag, expected_sha) = match std::env::consts::ARCH {
            "aarch64" => ("arm64", release.sha256_arm64.as_str()),
            "x86_64" => ("x86_64", release.sha256_x86_64.as_str()),
            other => anyhow::bail!("Unsupported architecture for Lima: {other}"),
        };

        let filename = format!("lima-{ver}-Darwin-{arch_tag}.tar.gz", ver = release.version);
        let urls = release.render_urls(&filename);

        tracing::info!("Installing private Lima {} into ~/.clawenv/ ...", release.version);

        let clawenv_root = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv");
        tokio::fs::create_dir_all(&clawenv_root).await?;

        let cache_dir = clawenv_root.join("cache");
        tokio::fs::create_dir_all(&cache_dir).await?;
        let tarball = cache_dir.join(&filename);

        let bytes = download_with_fallback(&urls, expected_sha).await?;
        tokio::fs::write(&tarball, &bytes).await
            .map_err(|e| anyhow!("Writing Lima tarball to cache failed: {e}"))?;

        // Lima tarball layout is `./bin/limactl` + `./share/lima/...`, so extracting
        // at ~/.clawenv/ puts the binary at ~/.clawenv/bin/limactl exactly.
        let status = Command::new("tar")
            .args([
                "xzf",
                &tarball.to_string_lossy(),
                "-C",
                &clawenv_root.to_string_lossy(),
            ])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("Failed to extract Lima tarball at {}", tarball.display());
        }

        let bin = limactl_bin();
        if !bin.exists() {
            anyhow::bail!(
                "Lima tarball extracted but {} is missing — unexpected archive layout",
                bin.display()
            );
        }

        // Best-effort: clear macOS quarantine attribute from the extracted tree
        // so Gatekeeper doesn't prompt on first launch. Non-fatal on failure —
        // curl-fetched tarballs don't set the attribute in the first place.
        let _ = Command::new("xattr")
            .args(["-dr", "com.apple.quarantine", &clawenv_root.to_string_lossy()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        tracing::info!("Lima {} installed at {}", release.version, bin.display());
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

                // The provision script carries its own multi-mirror fallback
                // chain; USER_ALPINE_MIRROR is just its highest-priority entry.
                // Leaving it empty is fine — the script falls through to the
                // canonical Fastly CDN and then to mainland-China mirrors.
                let rendered = template
                    .replace("{WORKSPACE_DIR}", &workspace_dir)
                    .replace("{GATEWAY_PORT}", &gateway_port.to_string())
                    .replace("{TTYD_PORT}", &ttyd_port.to_string())
                    .replace("{BRIDGE_PORT}", &bridge_port.to_string())
                    .replace("{CDP_PORT}", &cdp_port.to_string())
                    .replace("{VNC_WS_PORT}", &vnc_ws_port.to_string())
                    .replace("{PROXY_SCRIPT}", &opts.proxy_script)
                    .replace("{USER_ALPINE_MIRROR}", opts.alpine_mirror.trim_end_matches('/'));

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
        // --workdir /tmp prevents Lima from trying to cd to host CWD (which may not exist in VM)
        let args = ["shell", "--workdir", "/tmp", &self.vm_name, "--", "sh", "-c", cmd];
        let bin = limactl_bin();
        let (stdout, stderr, rc) = super::exec_helper::exec(&bin.to_string_lossy(), &args).await?;
        if rc != 0 {
            anyhow::bail!("exec failed (exit {rc}): {cmd}\nstdout: {}\nstderr: {}",
                stdout.chars().take(500).collect::<String>(),
                stderr.chars().take(500).collect::<String>());
        }
        Ok(stdout)
    }

    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String> {
        let args = ["shell", "--workdir", "/tmp", &self.vm_name, "--", "sh", "-c", cmd];
        let bin = limactl_bin();
        let (output, rc) = super::exec_helper::exec_with_progress(&bin.to_string_lossy(), &args, tx).await?;
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

        let lima_base = lima_home();
        tokio::fs::create_dir_all(&lima_base).await?;
        let vm_dir = lima_base.join(&self.vm_name);

        if vm_dir.exists() {
            anyhow::bail!(
                "Lima VM directory already exists at {}. Delete the existing instance \
                 first, or choose a different instance name when importing.",
                vm_dir.display()
            );
        }

        let clawenv_root = dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot find home directory"))?
            .join(".clawenv");
        let tmp_dir = clawenv_root.join(format!("_import_tmp_{}", std::process::id()));
        tokio::fs::create_dir_all(&tmp_dir).await?;

        let status = tokio::process::Command::new("tar")
            .args(["xzf", &path.to_string_lossy(), "-C", &tmp_dir.to_string_lossy()])
            .status()
            .await?;
        if !status.success() {
            tokio::fs::remove_dir_all(&tmp_dir).await.ok();
            anyhow::bail!("Failed to extract Lima image from {}", path.display());
        }

        // Two supported layouts:
        //   New: tarball root has `lima/<vm>/lima.yaml` + `bin/limactl` + `share/lima/`
        //   Old: tarball root has `<vm>/lima.yaml`
        // Find the directory holding lima.yaml in either layout.
        let src = match find_vm_dir_in_layout(&tmp_dir).await? {
            Some(p) => p,
            None => {
                tokio::fs::remove_dir_all(&tmp_dir).await.ok();
                anyhow::bail!(
                    "Extracted archive does not contain a Lima VM (no lima.yaml found in \
                     either tarball-root/<vm>/ or tarball-root/lima/<vm>/)."
                );
            }
        };

        // Move VM to final location with the target vm_name.
        tokio::fs::rename(&src, &vm_dir).await?;

        // New-layout bonus: if the tarball ships a Lima toolchain, seed our
        // private install only when the host doesn't already have one.
        let tmp_lima_bin = tmp_dir.join("bin").join("limactl");
        if tmp_lima_bin.exists() && !limactl_bin().exists() {
            let bin_dir = clawenv_root.join("bin");
            tokio::fs::create_dir_all(&bin_dir).await?;
            tokio::fs::rename(&tmp_lima_bin, bin_dir.join("limactl")).await.ok();
        }
        let tmp_share_lima = tmp_dir.join("share").join("lima");
        let host_share_lima = clawenv_root.join("share").join("lima");
        if tmp_share_lima.exists() && !host_share_lima.exists() {
            tokio::fs::create_dir_all(clawenv_root.join("share")).await?;
            tokio::fs::rename(&tmp_share_lima, &host_share_lima).await.ok();
        }

        tokio::fs::remove_dir_all(&tmp_dir).await.ok();

        // Rewrite absolute host paths in lima.yaml (mount locations) so the
        // imported VM targets this host's workspaces dir instead of the
        // exporter's home. Non-fatal if the file is unreadable — limactl start
        // will surface any real config problem.
        rewrite_lima_yaml_for_host(&vm_dir, &self.vm_name).await.ok();

        // Best-effort quarantine clear in case the tarball came through a
        // download that attached the attribute.
        let _ = Command::new("xattr")
            .args(["-dr", "com.apple.quarantine", &vm_dir.to_string_lossy()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

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

#[cfg(test)]
mod release_tests {
    use super::*;

    // Guards against a TOML layout regression: if `urls` is placed after
    // `[sha256]` it becomes `sha256.urls` and download fails at runtime.
    #[test]
    fn release_toml_parses_with_top_level_urls() {
        let r = LimaRelease::load().expect("lima-release.toml must parse");
        assert!(!r.version.is_empty(), "version missing");
        assert_eq!(r.sha256_arm64.len(), 64, "arm64 sha256 must be hex-64");
        assert_eq!(r.sha256_x86_64.len(), 64, "x86_64 sha256 must be hex-64");
        assert!(!r.url_templates.is_empty(), "urls must be a top-level array");

        let urls = r.render_urls("lima-x.y.z-Darwin-arm64.tar.gz");
        assert!(urls[0].contains(&r.version), "template {{version}} substitution failed");
        assert!(urls[0].contains("lima-x.y.z-Darwin-arm64.tar.gz"), "template {{filename}} substitution failed");
    }
}
