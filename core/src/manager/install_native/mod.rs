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
use crate::platform::download::download_with_progress;
use crate::config::{keychain, ConfigManager, InstanceConfig, GatewayConfig, ResourceConfig};
use crate::sandbox::{InstallMode, SandboxType, native_backend, SandboxBackend};

use super::install::{InstallOptions, InstallProgress, InstallStage, send, shell_escape};

// ---- Self-managed tool directories ----

/// ClawEnv-private Node.js directory (`<clawenv_root>/node/`).
pub fn clawenv_node_dir() -> std::path::PathBuf {
    crate::config::clawenv_root().join("node")
}

/// ClawEnv-private Git directory (`<clawenv_root>/git/`).
pub fn clawenv_git_dir() -> std::path::PathBuf {
    crate::config::clawenv_root().join("git")
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


/// Dugite platform key for the current host. Used as the sha256 lookup
/// key in `[dugite.sha256]` and as the URL template `{platform}` value.
/// Windows doesn't use dugite (it uses MinGit); callers should cfg-gate.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn dugite_current_platform() -> Result<&'static str> {
    let arch = std::env::consts::ARCH;
    #[cfg(target_os = "macos")]
    match arch {
        "aarch64" => Ok("macOS-arm64"),
        "x86_64"  => Ok("macOS-x64"),
        other => anyhow::bail!("Unsupported macOS architecture: {other}"),
    }
    #[cfg(target_os = "linux")]
    match arch {
        "aarch64" => Ok("ubuntu-arm64"),
        "x86_64"  => Ok("ubuntu-x64"),
        other => anyhow::bail!("Unsupported Linux architecture: {other}"),
    }
}

// Back-compat shim so the old call sites below still compile — inlined
// into install_git.
#[cfg(any(target_os = "macos", target_os = "linux"))]
struct GitRelease;
#[cfg(any(target_os = "macos", target_os = "linux"))]
impl GitRelease {
    fn load() -> Result<Self> { Ok(Self) }
    fn current_platform(&self) -> Result<(&'static str, String)> {
        let plat = dugite_current_platform()?;
        // sha256 lookup key (lowercase "macos-arm64" / "ubuntu-arm64") differs
        // from the URL platform key ("macOS-arm64" / "ubuntu-arm64") — normalise.
        let sha_key = plat.replace("macOS", "macos").replace("x64", "x86_64");
        let sha = crate::config::mirrors_asset::AssetMirrors::get()
            .expected_sha256("dugite", &sha_key)
            .ok_or_else(|| anyhow::anyhow!("mirrors.toml missing [dugite.sha256.{sha_key}]"))?;
        Ok((plat, sha))
    }
    fn render_urls(&self, platform: &str, proxy_on: bool) -> Result<Vec<(String, String)>> {
        crate::config::mirrors_asset::AssetMirrors::get().build_urls("dugite", platform, proxy_on)
    }
}

/// Install Git portable to ~/.clawenv/git/. Never depends on system git —
/// downloads a pinned binary release, verifies its sha256, and extracts.
/// The extracted tree is self-contained (bin/git + libexec/git-core + share/)
/// so bundle exports on this machine can be imported on any peer machine of
/// the same OS/arch without requiring system git on the target.
async fn install_git(tx: &mpsc::Sender<InstallProgress>, proxy_on: bool) -> Result<()> {
    let git_dir = clawenv_git_dir();
    let parent = git_dir.parent().unwrap_or(&git_dir).to_path_buf();
    tokio::fs::create_dir_all(&parent).await?;

    #[cfg(target_os = "windows")]
    {
        let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { "64-bit" };
        // URLs come from assets/mirrors.toml [mingit] — proxy_on drives
        // the official-only vs official+fallback tier selection.
        let urls = crate::config::mirrors_asset::AssetMirrors::get()
            .build_urls("mingit", arch, proxy_on)?;

        let bytes = download_with_progress(
            &urls, None, tx,
            InstallStage::EnsurePrerequisites,
            6, 8,
            &format!("Git for Windows (MinGit-{arch})"),
        ).await?;

        let zip_path = parent.join("git.zip");
        tokio::fs::write(&zip_path, &bytes).await?;

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
        let urls = release.render_urls(platform, proxy_on)?;
        let (_url, filename) = urls.first()
            .ok_or_else(|| anyhow::anyhow!("mirrors.toml [dugite] produced no URLs"))?;
        let filename = filename.clone();

        // Streaming download with throttled progress, per-chunk stall detection,
        // and sha256 verification. Mirror fallback is handled inside.
        let bytes = download_with_progress(
            &urls, Some(&expected_sha), tx,
            InstallStage::EnsurePrerequisites,
            6, 8,
            &format!("portable Git ({platform})"),
        ).await?;

        send(tx, "Extracting Git...", 8, InstallStage::EnsurePrerequisites).await;
        let tar_path = parent.join(&filename);
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

/// Build the URL list for Node.js downloads via the unified asset mirror
/// registry (`assets/mirrors.toml`). Platform keys: `darwin-arm64` /
/// `darwin-x64` / `win-arm64` / `win-x64`. Extension is `tar.gz` on unix,
/// `zip` on Windows.
///
/// The primary_base arg (from `mirrors.nodejs_dist_url()` config) is used
/// to prepend a user-pinned mirror ahead of the bundled list.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn build_node_urls(
    primary_base: &str,
    platform_with_ext: &str,
    proxy_on: bool,
) -> Result<Vec<(String, String)>> {
    use crate::config::mirrors_asset::AssetMirrors;
    // `platform_with_ext` is something like "darwin-arm64.tar.gz" or
    // "win-arm64.zip" — split on the last dot to separate platform from ext.
    let (platform, ext) = match platform_with_ext.rsplit_once('.') {
        Some((p, e)) if p.contains('-') => {
            // tar.gz → "darwin-arm64.tar" + "gz" — need to re-split.
            match p.rsplit_once('.') {
                Some((pp, "tar")) => (pp, format!("tar.{e}")),
                _ => (p, e.to_string()),
            }
        }
        _ => anyhow::bail!("malformed node platform key: {platform_with_ext}"),
    };
    let mirrors = AssetMirrors::get();
    // Proxy-aware URL list: proxy_on → official only; off → + fallback.
    let mut urls = mirrors.build_urls("node", platform, proxy_on)?;
    // Substitute {ext} in each URL manually since it's not a section scalar.
    for (u, _) in urls.iter_mut() {
        *u = u.replace("{ext}", &ext);
    }
    // User-pinned primary base takes precedence if non-empty.
    if !primary_base.is_empty()
        && primary_base != "https://nodejs.org/dist"
    {
        if let Some((first_url, first_file)) = urls.first().cloned() {
            // Rewrite the first url's base to primary_base.
            let replaced = first_url.replacen(
                "https://nodejs.org/dist",
                primary_base.trim_end_matches('/'),
                1,
            );
            urls.insert(0, (replaced, first_file));
        }
    }
    // Also fix the filename returned in the pair (loader embeds {ext} raw).
    for (_, f) in urls.iter_mut() {
        *f = f.replace("{ext}", &ext);
    }
    Ok(urls)
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

/// Host-side npm registry preflight (used by native install). HEAD each
/// candidate via reqwest with a short timeout; first 2xx/3xx wins.
/// Returns the first candidate on total failure so npm config at least
/// gets set to something sensible.
async fn select_reachable_npm_host(candidates: &[String]) -> Option<String> {
    // reqwest honours system / env proxy automatically; no explicit proxy
    // plumbing needed here — the installer-scope proxy will already be in
    // the process env thanks to apply_env() at install start.
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .connect_timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return candidates.first().cloned(),
    };
    for url in candidates {
        let probe = format!("{}/-/ping", url.trim_end_matches('/'));
        match client.head(&probe).send().await {
            Ok(resp) if resp.status().is_success() || resp.status().is_redirection() => {
                tracing::info!("npm preflight (native): {url} reachable");
                return Some(url.clone());
            }
            Ok(resp) => {
                tracing::info!("npm preflight (native): {url} returned {}", resp.status());
            }
            Err(e) => {
                tracing::info!("npm preflight (native): {url} unreachable: {e}");
            }
        }
    }
    candidates.first().cloned()
}

// ---- Platform-dispatched Node.js install ----

async fn install_nodejs(
    tx: &mpsc::Sender<InstallProgress>,
    nodejs_dist_base: &str,
    proxy_on: bool,
) -> Result<()> {
    #[cfg(target_os = "macos")]
    { macos::install_nodejs(tx, nodejs_dist_base, proxy_on).await }
    #[cfg(target_os = "windows")]
    { windows::install_nodejs(tx, nodejs_dist_base, proxy_on).await }
    #[cfg(target_os = "linux")]
    { linux::install_nodejs(tx, nodejs_dist_base, proxy_on).await }
}

/// Public API for CLI step-by-step install: install Node.js only.
pub async fn install_nodejs_public(
    tx: &mpsc::Sender<InstallProgress>,
    nodejs_dist_base: &str,
    proxy_on: bool,
) -> Result<()> {
    install_nodejs(tx, nodejs_dist_base, proxy_on).await
}

// ---- Main install flow (shared across platforms) ----

/// Native install flow — no VM, no MCP Bridge, no ttyd.
pub async fn install_native(
    opts: &InstallOptions,
    config: &mut ConfigManager,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    // Native: single instance, fixed directory. Honour CLAWENV_HOME so
    // E2E tests can isolate the install into a scratch dir.
    let install_dir = crate::config::clawenv_root().join("native");
    tokio::fs::create_dir_all(&install_dir).await?;

    // Dispatch: Bundle vs Online
    if let InstallMode::NativeBundle { ref path } = opts.install_mode {
        return install_from_bundle(opts, config, tx, path, &install_dir).await;
    }

    let mirrors = config.config().clawenv.mirrors.clone();

    // Install-time proxy snapshot — identical rationale to the sandbox
    // install path in manager/install.rs. One resolution at top of flow,
    // threaded down through every mirror consumer.
    let proxy_on = crate::config::proxy_resolver::Scope::Installer
        .resolve(config).await.is_some();

    // Step 0: Ensure Git (needed by npm for some dependencies)
    if !has_git().await {
        install_git(tx, proxy_on).await?;
    }

    // Step 1: Ensure Node.js
    send(tx, "Checking Node.js environment...", 10, InstallStage::EnsurePrerequisites).await;

    if !has_node().await {
        send(tx, "Node.js not found, installing...", 12, InstallStage::EnsurePrerequisites).await;
        let base = mirrors.nodejs_dist_urls(proxy_on).into_iter().next().unwrap_or_default();
        install_nodejs(tx, &base, proxy_on).await?;
        send(tx, "Node.js installed", 25, InstallStage::EnsurePrerequisites).await;
    } else {
        let ver = node_version().await;
        send(tx, &format!("Node.js {ver} ready"), 25, InstallStage::EnsurePrerequisites).await;
    }

    // Configure npm registry. Native mode's preflight is simple: try each
    // candidate from mirrors.toml (proxy-aware list) via a plain HEAD, pick
    // the first 2xx. Unlike sandbox mode we can use the host's reqwest
    // directly — no need to exec curl through a backend.
    let npm_candidates = mirrors.npm_registry_urls(proxy_on);
    let chosen = select_reachable_npm_host(&npm_candidates).await;
    if let Some(registry) = chosen.filter(|r| r != "https://registry.npmjs.org") {
        let shell = crate::platform::managed_shell::ManagedShell::new();
        shell.cmd(&format!("npm config set registry {registry}"))
            .status().await.ok();
        tracing::info!("npm registry set to {registry} (native)");
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

    // Step 5: Start gateway.
    // Uses `ManagedShell::spawn_detached` — the SAME path runtime
    // `start_instance` takes. Previously Windows went through an
    // ad-hoc `Start-Process -FilePath 'cmd.exe' -ArgumentList ...`
    // which inherited the wrong PATH, didn't redirect stdout/stderr
    // anywhere, and didn't properly detach → install claimed "gateway
    // started" but the process actually died before even writing a
    // log file. Going through spawn_detached fixes all three:
    //   - PATH is injected via the .bat wrapper
    //   - log path explicit (gateway.log)
    //   - PowerShell Start-Process with Hidden window = real detach
    send(tx, &format!("Starting {} gateway...", desc.display_name), 80, InstallStage::StartOpenClaw).await;
    let port = opts.gateway_port;
    if let Some(gateway_cmd) = desc.gateway_start_cmd(port) {
        let parts: Vec<&str> = gateway_cmd.split_whitespace().collect();
        let (bin, args) = if parts.len() > 1 { (parts[0], &parts[1..]) } else { (parts[0], &[][..]) };
        let log_path = crate::config::clawenv_root().join("native").join("gateway.log");
        let shell = crate::platform::managed_shell::ManagedShell::new();
        shell.spawn_detached(bin, args, &log_path).await?;
    }

    // Let the gateway bind its port before we tell the UI it's started.
    // Poll instead of a bare 2s sleep so slow Windows ARM64 boxes
    // (openclaw loads ~750 packages on each start) still get an honest
    // success signal and don't race the subsequent install stages.
    //
    // Budget: 120s total — openclaw cold-start on Windows ARM64 regularly
    // spends 30-60s on ESM module resolution before `listen()` even fires.
    // Earlier 30s window gave false negatives during E2E.
    let probe_port = opts.gateway_port;
    let mut gateway_up = false;
    for i in 0..40 {
        tokio::time::sleep(std::time::Duration::from_secs(if i == 0 { 2 } else { 3 })).await;
        if crate::monitor::InstanceMonitor::check_health_native(probe_port).await
            == crate::monitor::InstanceHealth::Running
        {
            gateway_up = true;
            break;
        }
    }
    if !gateway_up {
        // Surface the gateway log — install's "gateway started" claim
        // was silently bogus before this change; now we actually verify.
        let log_path = crate::config::clawenv_root().join("native").join("gateway.log");
        let log_tail = tokio::fs::read_to_string(&log_path).await.ok()
            .map(|s| s.lines().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|| "(no gateway.log produced — spawn likely failed to execute)".into());
        anyhow::bail!(
            "{} gateway failed to come up on port {probe_port} after ~120s.\n\nGateway log tail:\n{log_tail}",
            desc.display_name
        );
    }
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
        proxy: None,
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
        let log_path = crate::config::clawenv_root().join("native").join("gateway.log");
        let parts: Vec<&str> = gateway_cmd.split_whitespace().collect();
        let (bin, args) = if parts.len() > 1 { (parts[0], &parts[1..]) } else { (parts[0], &[][..]) };
        shell.spawn_detached(bin, args, &log_path).await?;
    }

    // Same port-readiness polling as the online-install path above —
    // refuse to claim "gateway started" until it actually listens.
    // 120s budget (Windows ARM64 cold-start).
    let probe_port = opts.gateway_port;
    let mut gateway_up = false;
    for i in 0..40 {
        tokio::time::sleep(std::time::Duration::from_secs(if i == 0 { 2 } else { 3 })).await;
        if crate::monitor::InstanceMonitor::check_health_native(probe_port).await
            == crate::monitor::InstanceHealth::Running
        {
            gateway_up = true;
            break;
        }
    }
    if !gateway_up {
        let log_path = crate::config::clawenv_root().join("native").join("gateway.log");
        let log_tail = tokio::fs::read_to_string(&log_path).await.ok()
            .map(|s| s.lines().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
            .unwrap_or_else(|| "(no gateway.log produced)".into());
        anyhow::bail!(
            "{} gateway failed to come up on port {probe_port} after ~120s.\n\nGateway log tail:\n{log_tail}",
            desc.display_name
        );
    }
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
        proxy: None,
        cached_latest_version: String::new(),
        cached_version_check_at: String::new(),
    })?;

    send(tx, "Installation complete! (from bundle)", 100, InstallStage::Complete).await;
    Ok(())
}

// Regression tests for the dugite URL path+filename split. Historically
// conflated the release tag ("2.53.0-3") with the upstream version ("2.53.0")
// and produced 404s. Now driven by `mirrors.toml` via `AssetMirrors`.
#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod git_release_tests {
    use crate::config::mirrors_asset::AssetMirrors;

    #[test]
    fn mirrors_toml_renders_dugite_url_correctly() {
        let m = AssetMirrors::get();
        let urls = m.build_urls("dugite", "macOS-arm64", false).expect("dugite urls");
        assert!(!urls.is_empty(), "urls array must have at least one entry");
        let (url, filename) = &urls[0];

        // Path must carry the full release tag "v2.53.0-3/"...
        assert!(
            url.contains("/download/v2.53.0-3/"),
            "URL should use release tag 'v2.53.0-3': got {url}"
        );
        // ...while filename uses upstream version "v2.53.0-"
        assert!(
            filename.contains("v2.53.0-"),
            "filename should use upstream version 'v2.53.0-': got {filename}"
        );
        // Regression guard: filename must NOT embed the build-counter suffix
        // "2.53.0-3" inside it (the exact 404 bug).
        assert!(
            !filename.contains("v2.53.0-3"),
            "filename must not contain tag 'v2.53.0-3': got {filename}"
        );
        assert!(filename.ends_with("-macOS-arm64.tar.gz"));

        // sha256 pinned
        let sha = m.expected_sha256("dugite", "macos-arm64").expect("sha256");
        assert_eq!(sha.len(), 64);
    }
}
