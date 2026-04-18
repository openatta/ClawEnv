//! Native install path — no VM, direct host installation.
//!
//! Platform-specific Node.js installation is in separate files:
//!   macos.rs   — Download .pkg, install via sudo installer
//!   windows.rs — Download .zip, extract to ~/.clawenv/node/ via PowerShell
//!   linux.rs   — Download .tar.xz, extract to ~/.clawenv/node/ via tar

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::claw::ClawRegistry;
use crate::config::{keychain, ConfigManager, InstanceConfig, GatewayConfig, ResourceConfig};
use crate::sandbox::{InstallMode, SandboxType, native_backend, SandboxBackend};

use super::install::{InstallOptions, InstallProgress, InstallStage, send, shell_escape};

// ---- Self-managed tool directories ----

/// ClawEnv-private Node.js directory (~/.clawenv/node/).
pub fn clawenv_node_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".clawenv").join("node")
}

/// ClawEnv-private Git directory (~/.clawenv/git/).
pub fn clawenv_git_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".clawenv").join("git")
}

/// Git binary path inside ClawEnv's private Git.
fn clawenv_git_bin() -> std::path::PathBuf {
    let dir = clawenv_git_dir();
    #[cfg(target_os = "windows")]
    { dir.join("cmd").join("git.exe") }
    #[cfg(target_os = "macos")]
    { dir.join("bin").join("git") }
    #[cfg(target_os = "linux")]
    { dir.join("bin").join("git") }
}

/// Check if ClawEnv's own Git is installed. Never uses system git.
pub async fn has_git() -> bool {
    clawenv_git_bin().exists()
}


/// Pinned dugite-native Git release metadata, loaded from the bundled TOML.
/// Version bumps happen by editing `assets/git/git-release.toml` — no code
/// change required.
///
/// Only used on macOS/Linux: Windows ships MinGit through a separate
/// hard-coded URL (see the `#[cfg(target_os = "windows")]` branch of
/// `install_git`). Gating the struct + impl here keeps the Windows build
/// under `-D warnings` without a blanket `#[allow(dead_code)]` that
/// would also hide accidental deadness on macOS/Linux.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[allow(dead_code)] // per-platform sha256 fields are only read under the matching #[cfg]
struct GitRelease {
    tag: String,                  // dugite release tag, e.g. "2.53.0-3" — URL path
    upstream_version: String,     // upstream git version, e.g. "2.53.0" — filename
    commit: String,
    url_templates: Vec<String>,
    sha256_macos_arm64: String,
    sha256_macos_x86_64: String,
    sha256_linux_arm64: String,
    sha256_linux_x86_64: String,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl GitRelease {
    fn load() -> Result<Self> {
        let src = include_str!("../../../../assets/git/git-release.toml");
        let t: toml::Table = src.parse()
            .map_err(|e| anyhow::anyhow!("git-release.toml invalid: {e}"))?;
        let tag = t.get("tag").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("git-release.toml missing `tag`"))?.to_string();
        let upstream_version = t.get("upstream_version").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("git-release.toml missing `upstream_version`"))?.to_string();
        let commit = t.get("commit").and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("git-release.toml missing `commit`"))?.to_string();
        let url_templates: Vec<String> = t.get("urls")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if url_templates.is_empty() {
            anyhow::bail!("git-release.toml must list at least one URL in top-level `urls`");
        }
        let sha = t.get("sha256").and_then(|v| v.as_table())
            .ok_or_else(|| anyhow::anyhow!("git-release.toml missing [sha256] table"))?;
        let get_sha = |k: &str| sha.get(k).and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("git-release.toml missing sha256.{k}"));
        Ok(Self {
            tag,
            upstream_version,
            commit,
            url_templates,
            sha256_macos_arm64:  get_sha("macos-arm64")?,
            sha256_macos_x86_64: get_sha("macos-x86_64")?,
            sha256_linux_arm64:  get_sha("linux-arm64")?,
            sha256_linux_x86_64: get_sha("linux-x86_64")?,
        })
    }

    /// Resolve (platform_tag, expected_sha256) for the current host, or bail
    /// if the target isn't supported (e.g. unknown arch).
    fn current_platform(&self) -> Result<(&'static str, &str)> {
        let arch = std::env::consts::ARCH;
        #[cfg(target_os = "macos")]
        {
            match arch {
                "aarch64" => Ok(("macOS-arm64", self.sha256_macos_arm64.as_str())),
                "x86_64"  => Ok(("macOS-x64",   self.sha256_macos_x86_64.as_str())),
                other => anyhow::bail!("Unsupported macOS architecture: {other}"),
            }
        }
        #[cfg(target_os = "linux")]
        {
            match arch {
                "aarch64" => Ok(("ubuntu-arm64", self.sha256_linux_arm64.as_str())),
                "x86_64"  => Ok(("ubuntu-x64",   self.sha256_linux_x86_64.as_str())),
                other => anyhow::bail!("Unsupported Linux architecture: {other}"),
            }
        }
        #[cfg(target_os = "windows")]
        { let _ = arch; anyhow::bail!("Git on Windows is shipped via MinGit, not dugite-native"); }
    }

    fn render_urls(&self, platform: &str) -> Vec<(String, String)> {
        // dugite asset filename uses upstream git version (e.g. "2.53.0"),
        // not the release tag ("2.53.0-3") — that's a dugite-specific
        // distinction. URL path in turn uses the release tag.
        let filename = format!(
            "dugite-native-v{ver}-{commit}-{platform}.tar.gz",
            ver = self.upstream_version, commit = self.commit, platform = platform,
        );
        self.url_templates.iter().map(|tmpl| {
            let u = tmpl
                .replace("{tag}",              &self.tag)
                .replace("{upstream_version}", &self.upstream_version)
                .replace("{commit}",           &self.commit)
                .replace("{platform}",         platform)
                .replace("{filename}",         &filename);
            (u, filename.clone())
        }).collect()
    }
}

/// Install Git portable to ~/.clawenv/git/. Never depends on system git —
/// downloads a pinned binary release, verifies its sha256, and extracts.
/// The extracted tree is self-contained (bin/git + libexec/git-core + share/)
/// so bundle exports on this machine can be imported on any peer machine of
/// the same OS/arch without requiring system git on the target.
async fn install_git(tx: &mpsc::Sender<InstallProgress>) -> Result<()> {
    let git_dir = clawenv_git_dir();
    let parent = git_dir.parent().unwrap_or(&git_dir).to_path_buf();
    tokio::fs::create_dir_all(&parent).await?;

    #[cfg(target_os = "windows")]
    {
        send(tx, "Downloading Git for Windows (MinGit)...", 6, InstallStage::EnsurePrerequisites).await;
        let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { "64-bit" };
        let url = format!(
            "https://github.com/git-for-windows/git/releases/download/v2.49.0.windows.1/MinGit-2.49.0-{arch}.zip"
        );
        let zip_path = parent.join("git.zip");

        let status = crate::platform::process::silent_cmd("curl.exe")
            .args(["-fSL", "-o", &zip_path.to_string_lossy(), &url])
            .status().await?;
        if !status.success() {
            anyhow::bail!("Failed to download MinGit from {url}");
        }

        send(tx, "Extracting Git...", 8, InstallStage::EnsurePrerequisites).await;
        let git_str = git_dir.to_string_lossy().replace('/', "\\");
        let zip_str = zip_path.to_string_lossy().replace('/', "\\");
        let cmd = format!(
            "Remove-Item -Recurse -Force '{}' -ErrorAction SilentlyContinue; \
             Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
            git_str, zip_str, git_str,
        );
        crate::platform::process::silent_cmd("powershell")
            .args(["-Command", &cmd])
            .status().await?;
        tokio::fs::remove_file(&zip_path).await.ok();

        if !clawenv_git_bin().exists() {
            anyhow::bail!("Git extraction failed: git.exe not found");
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let release = GitRelease::load()?;
        let (platform, expected_sha) = release.current_platform()?;
        let urls = release.render_urls(platform);
        let (_url, filename) = urls.first()
            .ok_or_else(|| anyhow::anyhow!("git-release.toml produced no URLs"))?;

        send(tx, &format!("Downloading portable Git ({platform})..."), 6, InstallStage::EnsurePrerequisites).await;

        // Fetch each URL in order, verifying sha256. Uses the same resilient
        // download pattern as the Lima installer.
        let bytes = download_git_tarball(&urls, expected_sha).await?;

        send(tx, "Extracting Git...", 8, InstallStage::EnsurePrerequisites).await;
        let tar_path = parent.join(filename);
        tokio::fs::write(&tar_path, &bytes).await?;

        // Dugite's tarball layout is a flat ./bin/ ./libexec/ ./share/ root
        // (no leading top-level directory), so extract straight into git_dir.
        let _ = tokio::fs::remove_dir_all(&git_dir).await;
        tokio::fs::create_dir_all(&git_dir).await?;
        let status = tokio::process::Command::new("tar")
            .args(["xzf", &tar_path.to_string_lossy(), "-C", &git_dir.to_string_lossy()])
            .status().await?;
        if let Err(e) = tokio::fs::remove_file(&tar_path).await {
            tracing::warn!("cleanup: failed to remove git tarball cache {}: {e}",
                tar_path.display());
        }
        if !status.success() {
            anyhow::bail!("Failed to extract git tarball into {}", git_dir.display());
        }

        // Best-effort quarantine clear on macOS so Gatekeeper doesn't prompt.
        #[cfg(target_os = "macos")]
        {
            match tokio::process::Command::new("xattr")
                .args(["-dr", "com.apple.quarantine", &git_dir.to_string_lossy()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status().await
            {
                Err(e) => tracing::warn!("xattr quarantine clear failed to spawn: {e}"),
                Ok(s) if !s.success() => tracing::warn!(
                    "xattr quarantine clear exited with {:?} on {} — Gatekeeper may \
                     prompt on first git invocation",
                    s.code(), git_dir.display()
                ),
                Ok(_) => {}
            }
        }

        if !clawenv_git_bin().exists() {
            anyhow::bail!(
                "Private git binary missing after extract: expected {}",
                clawenv_git_bin().display()
            );
        }
    }

    send(tx, "Git installed", 9, InstallStage::EnsurePrerequisites).await;
    Ok(())
}

/// Try each mirror URL in order, return the first response whose bytes match
/// the expected sha256. Mirrors checksum-mismatches to the next candidate so
/// a compromised mirror can't inject bad binaries — identical shape to the
/// Lima downloader.
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn download_git_tarball(urls: &[(String, String)], expected_sha256: &str) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};
    let mut last_err: Option<String> = None;
    for (url, _filename) in urls {
        tracing::info!("Trying git tarball URL: {url}");
        match reqwest::get(url).await {
            Err(e) => { last_err = Some(format!("{url}: {e}")); continue; }
            Ok(r) if !r.status().is_success() => {
                last_err = Some(format!("{url}: HTTP {}", r.status())); continue;
            }
            Ok(r) => match r.bytes().await {
                Err(e) => { last_err = Some(format!("{url}: body read: {e}")); continue; }
                Ok(bytes) => {
                    let hex = hex::encode(Sha256::digest(&bytes));
                    if hex == expected_sha256 {
                        return Ok(bytes.to_vec());
                    }
                    last_err = Some(format!("{url}: checksum mismatch"));
                }
            }
        }
    }
    anyhow::bail!(
        "All git download URLs failed. Last error: {}",
        last_err.as_deref().unwrap_or("(no URLs tried)")
    )
}

/// Node binary path inside ClawEnv's private Node.js.
fn clawenv_node_bin() -> std::path::PathBuf {
    let dir = clawenv_node_dir();
    #[cfg(target_os = "windows")]
    { dir.join("node.exe") }
    #[cfg(not(target_os = "windows"))]
    { dir.join("bin/node") }
}

/// Ensure ClawEnv's private Node.js is in this process's PATH.
pub fn ensure_node_in_path() {
    let dir = clawenv_node_dir();
    #[cfg(target_os = "windows")]
    let bin_dir = dir.clone();
    #[cfg(not(target_os = "windows"))]
    let bin_dir = dir.join("bin");

    let current = std::env::var("PATH").unwrap_or_default();
    let bin_str = bin_dir.to_string_lossy();
    if !current.contains(bin_str.as_ref()) {
        #[cfg(target_os = "windows")]
        let sep = ";";
        #[cfg(not(target_os = "windows"))]
        let sep = ":";
        std::env::set_var("PATH", format!("{}{sep}{current}", bin_dir.display()));
    }
}

/// Check if ClawEnv's own Node.js is installed. Never uses system node.
pub async fn has_node() -> bool {
    if clawenv_node_bin().exists() {
        ensure_node_in_path();
        return true;
    }
    false
}

async fn node_version() -> String {
    let shell = crate::platform::managed_shell::ManagedShell::new();
    shell.cmd("node --version")
        .output().await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

// ---- Platform-dispatched Node.js install ----

async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    { macos::install_nodejs(tx, nodejs_dist_base).await }
    #[cfg(target_os = "windows")]
    { windows::install_nodejs(tx, nodejs_dist_base).await }
    #[cfg(target_os = "linux")]
    { linux::install_nodejs(tx, nodejs_dist_base).await }
}

/// Public API for CLI step-by-step install: install Node.js only.
pub async fn install_nodejs_public(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    install_nodejs(tx, nodejs_dist_base).await
}

// ---- Main install flow (shared across platforms) ----

/// Native install flow — no VM, no MCP Bridge, no ttyd.
pub async fn install_native(
    opts: &InstallOptions,
    config: &mut ConfigManager,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    // Native: single instance, fixed directory
    let install_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".clawenv")
        .join("native");
    tokio::fs::create_dir_all(&install_dir).await?;

    // Dispatch: Bundle vs Online
    if let InstallMode::NativeBundle { ref path } = opts.install_mode {
        return install_from_bundle(opts, config, tx, path, &install_dir).await;
    }

    let mirrors = &config.config().clawenv.mirrors;

    // Step 0: Ensure Git (needed by npm for some dependencies)
    if !has_git().await {
        install_git(tx).await?;
    }

    // Step 1: Ensure Node.js
    send(tx, "Checking Node.js environment...", 10, InstallStage::EnsurePrerequisites).await;

    if !has_node().await {
        send(tx, "Node.js not found, installing...", 12, InstallStage::EnsurePrerequisites).await;
        install_nodejs(tx, mirrors.nodejs_dist_url()).await?;
        send(tx, "Node.js installed", 25, InstallStage::EnsurePrerequisites).await;
    } else {
        let ver = node_version().await;
        send(tx, &format!("Node.js {ver} ready"), 25, InstallStage::EnsurePrerequisites).await;
    }

    // Configure npm registry mirror — use our managed npm
    let npm_registry = mirrors.npm_registry_url();
    if npm_registry != "https://registry.npmjs.org" {
        let shell = crate::platform::managed_shell::ManagedShell::new();
        shell.cmd(&format!("npm config set registry {npm_registry}"))
            .status().await.ok();
    }

    // Step 2: Install claw product
    let registry = ClawRegistry::load();
    let desc = registry.get(&opts.claw_type);
    let backend = native_backend(&opts.instance_name);

    let claw_installed = backend.exec(&format!("{} 2>/dev/null || echo ''", desc.version_check_cmd())).await
        .map(|o| !o.trim().is_empty()).unwrap_or(false);

    if !claw_installed {
        send(tx, &format!("Installing {}...", desc.display_name), 30, InstallStage::InstallOpenClaw).await;

        let (progress_tx, mut progress_rx) = mpsc::channel::<String>(64);
        let tx_ui = tx.clone();
        let ui_task = tokio::spawn(async move {
            let start = std::time::Instant::now();
            while let Some(line) = progress_rx.recv().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let elapsed = start.elapsed().as_secs();
                    let short = if trimmed.len() > 80 { &trimmed[..80] } else { trimmed };
                    let pct = std::cmp::min(30 + (elapsed / 10) as u8, 65);
                    send(&tx_ui, &format!("[{elapsed}s] {short}"), pct, InstallStage::InstallOpenClaw).await;
                }
            }
        });

        let result = backend.exec_with_progress(
            &desc.npm_install_prefix_cmd(&opts.claw_version, &install_dir.to_string_lossy()),
            &progress_tx,
        ).await;
        drop(progress_tx);
        ui_task.await.ok();
        result?;

        send(tx, &format!("{} installed", desc.display_name), 68, InstallStage::InstallOpenClaw).await;
    } else {
        send(tx, &format!("{} already installed", desc.display_name), 68, InstallStage::InstallOpenClaw).await;
    }

    let claw_version = backend.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await.unwrap_or_default();

    // Step 3: API Key
    if let Some(ref api_key) = opts.api_key {
        send(tx, "Storing API key...", 72, InstallStage::StoreApiKey).await;
        keychain::store_api_key(&opts.instance_name, api_key)?;
        if let Some(cmd) = desc.set_apikey_cmd(&shell_escape(api_key)) {
            backend.exec(&format!("{cmd} 2>/dev/null || true")).await?;
        }
    }

    // Step 4: Deploy MCP plugins (native mode — same plugins as sandbox)
    if desc.supports_mcp {
        send(tx, "Installing MCP plugins...", 75, InstallStage::StartOpenClaw).await;
        let mcp_base = install_dir.join("mcp");
        let bridge_url = format!("http://127.0.0.1:{}", crate::manager::install::allocate_port(opts.gateway_port, 2));
        let use_python = desc.uses_python_mcp();

        // Deploy scripts based on agent runtime (Node.js or Python)
        let plugins: Vec<(&str, &str, &str)> = if use_python {
            vec![
                ("mcp-bridge", "bridge.py", include_str!("../../../../assets/mcp/mcp-bridge.py")),
                ("hil-skill", "skill.py", include_str!("../../../../assets/mcp/hil-skill.py")),
                ("hw-notify", "notify.py", include_str!("../../../../assets/mcp/hw-notify.py")),
            ]
        } else {
            vec![
                ("mcp-bridge", "index.mjs", include_str!("../../../../assets/mcp/mcp-bridge.mjs")),
                ("hil-skill", "index.mjs", include_str!("../../../../assets/mcp/hil-skill.mjs")),
                ("hw-notify", "index.mjs", include_str!("../../../../assets/mcp/hw-notify.mjs")),
            ]
        };

        for (dir_name, file_name, content) in &plugins {
            let dir = mcp_base.join(dir_name);
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::write(dir.join(file_name), content).await?;
        }

        // Register all MCP plugins with the agent
        let runner = if use_python { "python3" } else { "node" };
        let plugin_entries: Vec<(&str, std::path::PathBuf)> = plugins.iter()
            .map(|(dir_name, file_name, _)| {
                let reg_name = match *dir_name {
                    "mcp-bridge" => "clawenv",
                    "hil-skill" => "clawenv-hil",
                    _ => *dir_name,
                };
                (reg_name, mcp_base.join(dir_name).join(file_name))
            })
            .collect();

        for (name, entry) in &plugin_entries {
            if let Some(cmd) = desc.mcp_register_cmd(
                name,
                &format!("{{\"command\":\"{runner}\",\"args\":[\"{}\",\"--bridge-url\",\"{bridge_url}\"]}}", entry.display()),
            ) {
                backend.exec(&format!("{cmd} 2>/dev/null || true")).await.ok();
            }
        }
        tracing::info!("MCP plugins ({runner}) deployed to {}", mcp_base.display());
    }

    // Step 5: Start gateway
    send(tx, &format!("Starting {} gateway...", desc.display_name), 80, InstallStage::StartOpenClaw).await;
    let port = opts.gateway_port;
    if let Some(gateway_cmd) = desc.gateway_start_cmd(port) {
        // Instance name is validated (alphanumeric + dash + underscore),
        // safe for paths. Unix wants single-quote escaping for the log
        // path literal; Windows uses a different invocation that doesn't
        // interpolate instance_name — compute each inside the cfg that
        // actually uses it.
        #[cfg(not(target_os = "windows"))]
        {
            let name_esc = opts.instance_name.replace('\'', "'\\''");
            backend.exec(&format!(
                "nohup {gateway_cmd} > '/tmp/clawenv-gateway-{name_esc}.log' 2>&1 &"
            )).await?;
        }
        #[cfg(target_os = "windows")]
        {
            let full_cmd = gateway_cmd.replace('\'', "''");
            backend.exec(&format!(
                "Start-Process -WindowStyle Hidden -FilePath 'cmd.exe' -ArgumentList '/c {full_cmd}'"
            )).await?;
        }
    }

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    send(tx, &format!("{} gateway started", desc.display_name), 85, InstallStage::StartOpenClaw).await;

    // Step 5: Save config
    send(tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    config.save_instance(InstanceConfig {
        name: opts.instance_name.clone(),
        claw_type: opts.claw_type.clone(),
        version: claw_version.trim().to_string(),
        sandbox_type: SandboxType::Native,
        sandbox_id: "native".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_upgraded_at: String::new(),
        gateway: GatewayConfig {
            gateway_port: opts.gateway_port,
            ttyd_port: 0,
            bridge_port: crate::manager::install::allocate_port(opts.gateway_port, 2),
            // Native Hermes isn't supported yet (supports_native=false in
            // registry), so in practice this branch only ever runs for
            // OpenClaw — but we still honor has_dashboard() for future claws.
            dashboard_port: if desc.has_dashboard() {
                crate::manager::install::allocate_port(opts.gateway_port, desc.dashboard_port_offset)
            } else { 0 },
            webchat_enabled: true,
            channels: Default::default(),
        },
        resources: ResourceConfig::default(),
        browser: Default::default(),
        cached_latest_version: String::new(),
        cached_version_check_at: String::new(),
    })?;

    send(tx, "Installation complete!", 100, InstallStage::Complete).await;
    Ok(())
}

// ---- Bundle install (shared, with platform-specific extraction) ----

async fn install_from_bundle(
    opts: &InstallOptions,
    config: &mut ConfigManager,
    tx: &mpsc::Sender<InstallProgress>,
    bundle_path: &std::path::Path,
    install_dir: &std::path::Path,
) -> Result<()> {
    if !bundle_path.exists() {
        anyhow::bail!("Bundle file not found: {}", bundle_path.display());
    }

    // Validate the manifest BEFORE extracting. v0.2.6+ bundles carry a
    // `clawenv-bundle.toml` at archive root — we insist on it being present
    // and identifying a native-produced bundle so we never untar, say, a
    // Lima or WSL bundle into ~/.clawenv/ and end up with mixed state. The
    // old string-search over `manifest.toml` was both too loose (matched
    // partial words) and too late (ran after extract).
    let manifest = crate::export::BundleManifest::peek_from_tarball(bundle_path).await
        .map_err(|e| anyhow::anyhow!(
            "Cannot import native bundle from {}: {e}\n\nBundles produced by \
             pre-v0.2.6 clawenv are no longer supported — re-export from the \
             source with a current clawenv build.",
            bundle_path.display()
        ))?;
    if manifest.sandbox_type != crate::sandbox::SandboxType::Native.as_wire_str() {
        anyhow::bail!(
            "Bundle {} declares sandbox_type '{}' but this machine uses a native install. \
             Don't cross-import sandbox bundles into a native setup.",
            bundle_path.display(), manifest.sandbox_type
        );
    }
    // Matching claw_type isn't strictly required (the user might be
    // reinstalling a different claw over this native install), but mixing
    // typically leads to broken gateway startup — warn rather than block.
    if manifest.claw_type != opts.claw_type {
        tracing::warn!(
            "Bundle is for claw_type '{}' but install options request '{}'; \
             proceeding but the installed gateway may not match expectations.",
            manifest.claw_type, opts.claw_type
        );
    }

    send(tx, "Extracting native bundle...", 10, InstallStage::EnsurePrerequisites).await;

    // The bundle layout produced by v0.2.6+ has node/git/native at archive
    // root alongside the manifest. install_dir is ~/.clawenv/native, but
    // node/ and git/ live one level up at ~/.clawenv/{node,git}. So we
    // extract into ~/.clawenv (install_dir.parent()), not install_dir
    // itself — otherwise node/git land nested and nothing finds them.
    let extract_root = install_dir.parent().unwrap_or(install_dir);
    let dest = extract_root.to_string_lossy().to_string();
    let src = bundle_path.to_string_lossy().to_string();

    // Platform-specific extraction
    #[cfg(not(target_os = "windows"))]
    {
        let status = tokio::process::Command::new("tar")
            .args(["xzf", &src, "-C", &dest])
            .status().await?;
        if !status.success() {
            anyhow::bail!("Failed to extract bundle");
        }
    }
    #[cfg(target_os = "windows")]
    {
        let status = crate::platform::process::silent_cmd("tar")
            .args(["xzf", &src, "-C", &dest])
            .status().await;
        if !status.map(|s| s.success()).unwrap_or(false) {
            anyhow::bail!("Failed to extract bundle. Ensure Windows 10+ with built-in tar.");
        }
    }
    // The manifest copy that got extracted along with the payload is
    // redundant on disk — strip it so ~/.clawenv doesn't accumulate stale
    // sidecar files across multiple imports.
    let _ = tokio::fs::remove_file(
        extract_root.join(crate::export::manifest::MANIFEST_FILENAME)
    ).await;

    send(tx, "Bundle extracted", 30, InstallStage::EnsurePrerequisites).await;
    send(tx, "Bundle validated", 40, InstallStage::EnsurePrerequisites).await;

    // Setup PATH
    #[cfg(not(target_os = "windows"))]
    let (node_bin, modules_bin) = (install_dir.join("node/bin"), install_dir.join("node_modules/.bin"));
    #[cfg(target_os = "windows")]
    let (node_bin, modules_bin) = (install_dir.join("node"), install_dir.join("node_modules/.bin"));

    ensure_node_in_path();
    let current_path = std::env::var("PATH").unwrap_or_default();
    #[cfg(target_os = "windows")]
    std::env::set_var("PATH", format!("{};{};{current_path}", node_bin.display(), modules_bin.display()));
    #[cfg(not(target_os = "windows"))]
    std::env::set_var("PATH", format!("{}:{}:{current_path}", node_bin.display(), modules_bin.display()));

    // Verify
    let backend = native_backend(&opts.instance_name);
    let registry = ClawRegistry::load();
    let desc = registry.get(&opts.claw_type);

    let claw_ok = backend.exec(&desc.version_check_cmd()).await;
    if claw_ok.is_err() {
        anyhow::bail!("Bundle does not contain {} — invalid bundle", desc.display_name);
    }
    let oc_version = claw_ok.unwrap_or_default().trim().to_string();
    send(tx, &format!("{} {oc_version} ready (from bundle)", desc.display_name), 68, InstallStage::InstallOpenClaw).await;

    // API Key
    if let Some(ref api_key) = opts.api_key {
        send(tx, "Storing API key...", 72, InstallStage::StoreApiKey).await;
        keychain::store_api_key(&opts.instance_name, api_key)?;
        if let Some(cmd) = desc.set_apikey_cmd(&shell_escape(api_key)) {
            backend.exec(&format!("{cmd} 2>/dev/null || true")).await?;
        }
    }

    // Start gateway via ManagedShell::spawn_detached (works on all platforms)
    if let Some(gateway_cmd) = desc.gateway_start_cmd(opts.gateway_port) {
        send(tx, &format!("Starting {} gateway...", desc.display_name), 80, InstallStage::StartOpenClaw).await;
        let shell = crate::platform::managed_shell::ManagedShell::new();
        let log_path = dirs::home_dir().unwrap_or_default()
            .join(".clawenv").join("native").join("gateway.log");
        let parts: Vec<&str> = gateway_cmd.split_whitespace().collect();
        let (bin, args) = if parts.len() > 1 { (parts[0], &parts[1..]) } else { (parts[0], &[][..]) };
        shell.spawn_detached(bin, args, &log_path).await?;
    }

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    send(tx, &format!("{} gateway started", desc.display_name), 85, InstallStage::StartOpenClaw).await;

    // Save config
    send(tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    config.save_instance(InstanceConfig {
        name: opts.instance_name.clone(),
        claw_type: opts.claw_type.clone(),
        version: oc_version,
        sandbox_type: SandboxType::Native,
        sandbox_id: "native".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_upgraded_at: String::new(),
        gateway: GatewayConfig {
            gateway_port: opts.gateway_port,
            ttyd_port: 0,
            bridge_port: crate::manager::install::allocate_port(opts.gateway_port, 2),
            dashboard_port: if desc.has_dashboard() {
                crate::manager::install::allocate_port(opts.gateway_port, desc.dashboard_port_offset)
            } else { 0 },
            webchat_enabled: true,
            channels: Default::default(),
        },
        resources: ResourceConfig::default(),
        browser: Default::default(),
        cached_latest_version: String::new(),
        cached_version_check_at: String::new(),
    })?;

    send(tx, "Installation complete! (from bundle)", 100, InstallStage::Complete).await;
    Ok(())
}

// The test module references `GitRelease` which is cfg-gated to
// macos/linux. Gate the tests the same way so the Windows build doesn't
// drag in a non-existent symbol.
#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod git_release_tests {
    use super::GitRelease;

    // Regression guard: dugite-native's release tag (e.g. "2.53.0-3") carries
    // a build-counter suffix that is NOT present in asset filenames ("2.53.0").
    // Conflating the two produces URLs like /download/v2.53.0-3/dugite-...-
    // v2.53.0-3-f49d009-macOS-arm64.tar.gz which 404 on the server.
    #[test]
    fn release_toml_renders_dugite_url_correctly() {
        let release = GitRelease::load().expect("git-release.toml must parse");
        assert!(!release.tag.is_empty(), "tag must not be empty");
        assert!(!release.upstream_version.is_empty(), "upstream_version must not be empty");
        assert!(!release.commit.is_empty(), "commit must not be empty");
        assert_eq!(release.sha256_macos_arm64.len(), 64);

        let urls = release.render_urls("macOS-arm64");
        assert!(!urls.is_empty(), "urls array must have at least one entry");
        let (url, filename) = &urls[0];

        // Path must carry the full release tag…
        assert!(
            url.contains(&format!("/download/v{}/", release.tag)),
            "URL path should use release tag 'v{}' but got: {url}",
            release.tag
        );
        // …while the asset filename uses only the upstream git version.
        assert!(
            filename.contains(&format!("v{}-", release.upstream_version)),
            "filename should use upstream_version 'v{}' but got: {filename}",
            release.upstream_version
        );
        // And must NOT embed the tag's build-counter suffix inside the filename,
        // which was the exact regression that produced 404s.
        assert!(
            !filename.contains(&release.tag) || release.tag == release.upstream_version,
            "filename must not contain the full dugite tag '{}' — that was the 404 bug. \
             Got: {filename}",
            release.tag
        );
        assert!(filename.ends_with("-macOS-arm64.tar.gz"));
    }
}
