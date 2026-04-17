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

/// Install Git portable to ~/.clawenv/git/. Never depends on system git.
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

    #[cfg(target_os = "macos")]
    {
        send(tx, "Downloading Git for macOS...", 6, InstallStage::EnsurePrerequisites).await;
        let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { "x86_64" };
        // git-manpages-free: standalone portable git built for macOS
        let url = format!(
            "https://github.com/nicknisi/git-for-mac/releases/latest/download/git-macos-{arch}.tar.gz"
        );
        let tar_path = parent.join("git.tar.gz");

        let status = tokio::process::Command::new("curl")
            .args(["-fSL", "-o", &tar_path.to_string_lossy(), &url])
            .status().await?;

        if !status.success() {
            // Fallback: try git-scm.com universal binary
            let url2 = "https://sourceforge.net/projects/git-osx-installer/files/latest/download";
            let _ = tokio::process::Command::new("curl")
                .args(["-fSL", "-o", &tar_path.to_string_lossy(), "-L", url2])
                .status().await;
        }

        send(tx, "Extracting Git...", 8, InstallStage::EnsurePrerequisites).await;
        let _ = tokio::fs::remove_dir_all(&git_dir).await;
        tokio::fs::create_dir_all(&git_dir).await?;
        tokio::process::Command::new("tar")
            .args(["xzf", &tar_path.to_string_lossy(), "--strip-components=1", "-C", &git_dir.to_string_lossy()])
            .status().await?;
        tokio::fs::remove_file(&tar_path).await.ok();

        // If portable git doesn't have expected structure, create bin/ symlink to system git as last resort
        if !clawenv_git_bin().exists() {
            tokio::fs::create_dir_all(git_dir.join("bin")).await?;
            // Try to find any git binary in extracted dir
            let find = tokio::process::Command::new("find")
                .args([&git_dir.to_string_lossy().to_string(), "-name", "git", "-type", "f"])
                .output().await;
            if let Ok(out) = find {
                let found = String::from_utf8_lossy(&out.stdout);
                if let Some(path) = found.lines().next() {
                    let _ = tokio::fs::symlink(path, git_dir.join("bin").join("git")).await;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        send(tx, "Git not found. Please install: sudo apt install git", 8, InstallStage::EnsurePrerequisites).await;
        anyhow::bail!("Git is required but not installed. Run: sudo apt install git (Ubuntu/Debian) or sudo dnf install git (Fedora)");
    }

    send(tx, "Git installed", 9, InstallStage::EnsurePrerequisites).await;
    Ok(())
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
        // Instance name is validated (alphanumeric + dash + underscore), safe for paths.
        let name_esc = opts.instance_name.replace('\'', "'\\''");
        #[cfg(not(target_os = "windows"))]
        backend.exec(&format!(
            "nohup {gateway_cmd} > '/tmp/clawenv-gateway-{name_esc}.log' 2>&1 &"
        )).await?;
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

    send(tx, "Extracting native bundle...", 10, InstallStage::EnsurePrerequisites).await;

    let dest = install_dir.to_string_lossy().to_string();
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

    send(tx, "Bundle extracted", 30, InstallStage::EnsurePrerequisites).await;

    // Validate manifest
    let manifest_path = install_dir.join("manifest.toml");
    if manifest_path.exists() {
        let manifest_str = tokio::fs::read_to_string(&manifest_path).await.unwrap_or_default();
        let expected_platform = std::env::consts::OS;
        let expected_arch = match std::env::consts::ARCH { "x86_64" => "x64", "aarch64" => "arm64", other => other };
        let ok = manifest_str.lines().any(|l| l.contains(expected_platform))
            && manifest_str.lines().any(|l| l.contains(expected_arch));
        if !ok {
            anyhow::bail!("Bundle platform mismatch: expected {}-{}", expected_platform, expected_arch);
        }
    }

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
        gateway: GatewayConfig { gateway_port: opts.gateway_port, ttyd_port: 0, bridge_port: crate::manager::install::allocate_port(opts.gateway_port, 2), webchat_enabled: true, channels: Default::default() },
        resources: ResourceConfig::default(),
        browser: Default::default(),
        cached_latest_version: String::new(),
        cached_version_check_at: String::new(),
    })?;

    send(tx, "Installation complete! (from bundle)", 100, InstallStage::Complete).await;
    Ok(())
}
