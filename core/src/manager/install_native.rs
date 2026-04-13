//! Native install path — no VM, direct host installation.
//!
//! Platform-specific Node.js installation:
//!   macOS:   Download .pkg from nodejs.org, `installer` command (needs admin password)
//!   Windows: Download .msi, `msiexec /i /qn` (needs UAC)
//!   Linux:   Download tar.xz to ~/.clawenv/node/, add to PATH

use anyhow::Result;
use tokio::sync::mpsc;

use crate::claw::ClawRegistry;
use crate::config::{keychain, ConfigManager, InstanceConfig, GatewayConfig, ResourceConfig};
use crate::sandbox::{InstallMode, SandboxType, native_backend, SandboxBackend};

use super::install::{InstallOptions, InstallProgress, InstallStage, send, shell_escape};

/// Native install flow — no VM, no MCP Bridge, no ttyd.
pub async fn install_native(
    opts: &InstallOptions,
    config: &mut ConfigManager,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    let install_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".clawenv/native")
        .join(&opts.instance_name);
    tokio::fs::create_dir_all(&install_dir).await?;

    // ---- Dispatch: Bundle vs Online ----
    if let InstallMode::NativeBundle { ref path } = opts.install_mode {
        return install_from_bundle(opts, config, tx, path, &install_dir).await;
    }

    let mirrors = &config.config().clawenv.mirrors;

    // ---- Online path: Step 1 — Ensure Node.js + npm ----
    send(tx, "Checking Node.js environment...", 10, InstallStage::EnsurePrerequisites).await;

    if !has_node().await {
        send(tx, "Node.js not found, installing...", 12, InstallStage::EnsurePrerequisites).await;
        install_nodejs(tx, mirrors.nodejs_dist_url()).await?;
        send(tx, "Node.js installed", 25, InstallStage::EnsurePrerequisites).await;
    } else {
        let ver = node_version().await;
        send(tx, &format!("Node.js {ver} ready"), 25, InstallStage::EnsurePrerequisites).await;
    }

    // Configure npm registry mirror if needed
    let npm_registry = mirrors.npm_registry_url();
    if npm_registry != "https://registry.npmjs.org" {
        let _ = crate::platform::process::silent_cmd("npm")
            .args(["config", "set", "registry", npm_registry])
            .status().await;
    }

    // ---- Step 2: Install claw product ----
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
            &desc.npm_install_verbose_cmd(&opts.claw_version),
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

    // ---- Step 3: API Key ----
    if let Some(ref api_key) = opts.api_key {
        send(tx, "Storing API key...", 72, InstallStage::StoreApiKey).await;
        keychain::store_api_key(&opts.instance_name, api_key)?;
        if let Some(cmd) = desc.set_apikey_cmd(&shell_escape(api_key)) {
            backend.exec(&format!("{cmd} 2>/dev/null || true")).await?;
        }
    }

    // ---- Step 4: Start gateway ----
    send(tx, &format!("Starting {} gateway...", desc.display_name), 80, InstallStage::StartOpenClaw).await;
    let port = opts.gateway_port;
    let gateway_cmd = desc.gateway_start_cmd(port);
    backend.exec(&format!(
        "nohup {gateway_cmd} > /tmp/clawenv-gateway-{}.log 2>&1 &",
        opts.instance_name
    )).await?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    send(tx, &format!("{} gateway started", desc.display_name), 85, InstallStage::StartOpenClaw).await;

    // ---- Step 5: Save config ----
    // Native: no MCP Bridge, no ttyd, no host IP sync
    send(tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    config.config_mut().instances.retain(|i| i.name != opts.instance_name);
    config.config_mut().instances.push(InstanceConfig {
        name: opts.instance_name.clone(),
        claw_type: opts.claw_type.clone(),
        version: claw_version.trim().to_string(),
        sandbox_type: SandboxType::Native,
        sandbox_id: format!("native-{}", opts.instance_name),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_upgraded_at: String::new(),
        gateway: GatewayConfig {
            gateway_port: opts.gateway_port,
            ttyd_port: 0, // no ttyd for native
            webchat_enabled: true,
            channels: Default::default(),
        },
        resources: ResourceConfig::default(),
        browser: Default::default(),
        cached_latest_version: String::new(),
        cached_version_check_at: String::new(),
    });
    config.save()?;

    send(tx, "Installation complete!", 100, InstallStage::Complete).await;
    Ok(())
}

// ---- Native Bundle install ----

/// Install from a pre-packaged native bundle (tar.gz containing node/ + node_modules/).
///
/// Bundle layout:
///   bundle.tar.gz
///   ├── node/          — Node.js runtime (bin/, lib/, include/)
///   ├── node_modules/  — Pre-installed global packages (openclaw, etc.)
///   └── manifest.toml  — Bundle metadata (version, platform, arch)
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

    // Extract to install_dir (platform-aware)
    let dest = install_dir.to_string_lossy().to_string();
    let src = bundle_path.to_string_lossy().to_string();

    #[cfg(not(target_os = "windows"))]
    {
        let status = tokio::process::Command::new("tar")
            .args(["xzf", &src, "-C", &dest])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("Failed to extract bundle: {}", bundle_path.display());
        }
    }
    #[cfg(target_os = "windows")]
    {
        // Windows: use tar (available on Windows 10+) or PowerShell fallback
        let status = tokio::process::Command::new("tar")
            .args(["xzf", &src, "-C", &dest])
            .status()
            .await;
        match status {
            Ok(s) if s.success() => {}
            _ => {
                // Fallback: PowerShell Expand-Archive (requires .zip, but tar.gz should work with tar)
                anyhow::bail!(
                    "Failed to extract bundle. Ensure Windows 10+ with built-in tar, or extract manually:\n\
                     tar xzf \"{}\" -C \"{}\"", src, dest
                );
            }
        }
    }

    send(tx, "Bundle extracted", 30, InstallStage::EnsurePrerequisites).await;

    // ---- Validate platform/arch from manifest ----
    send(tx, "Validating bundle manifest...", 35, InstallStage::EnsurePrerequisites).await;
    let manifest_path = install_dir.join("manifest.toml");
    if manifest_path.exists() {
        let manifest_str = tokio::fs::read_to_string(&manifest_path).await.unwrap_or_default();
        let expected_platform = match std::env::consts::OS {
            "macos" => "macos",
            "linux" => "linux",
            "windows" => "windows",
            other => other,
        };
        let expected_arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            other => other,
        };
        // Simple TOML key check (avoid pulling in a TOML parser just for this)
        let has_platform = manifest_str.lines().any(|l| {
            let l = l.trim();
            l.starts_with("platform") && l.contains(expected_platform)
        });
        let has_arch = manifest_str.lines().any(|l| {
            let l = l.trim();
            l.starts_with("arch") && l.contains(expected_arch)
        });
        if !has_platform || !has_arch {
            anyhow::bail!(
                "Bundle platform mismatch: expected {}-{}, check manifest.toml",
                expected_platform, expected_arch
            );
        }
    }

    send(tx, "Bundle validated", 40, InstallStage::EnsurePrerequisites).await;

    // Setup PATH: add bundled node/bin and node_modules/.bin
    #[cfg(not(target_os = "windows"))]
    let (node_bin, modules_bin, path_sep) = (
        install_dir.join("node/bin"),
        install_dir.join("node_modules/.bin"),
        ":",
    );
    #[cfg(target_os = "windows")]
    let (node_bin, modules_bin, path_sep) = (
        install_dir.join("node"),       // Windows node binary is in node/ directly
        install_dir.join("node_modules/.bin"),
        ";",
    );

    let current_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!(
        "{}{path_sep}{}{path_sep}{current_path}",
        node_bin.display(), modules_bin.display()
    ));

    // Verify node + openclaw are available
    send(tx, "Verifying bundle contents...", 50, InstallStage::InstallOpenClaw).await;

    let backend = native_backend(&opts.instance_name);

    #[cfg(not(target_os = "windows"))]
    let path_export = format!(
        "export PATH=\"{}:{}:$PATH\"",
        node_bin.display(), modules_bin.display()
    );
    #[cfg(target_os = "windows")]
    let path_export = format!(
        "set PATH={};{};%PATH%",
        node_bin.display(), modules_bin.display()
    );

    // Resolve descriptor for bundle verification
    let bundle_registry = ClawRegistry::load();
    let bundle_desc = bundle_registry.get(&opts.claw_type);

    let node_ok = backend.exec(&format!(
        "{path_export} && node --version"
    )).await;
    if node_ok.is_err() {
        anyhow::bail!("Bundle does not contain a valid Node.js runtime");
    }

    let claw_ok = backend.exec(&format!(
        "{path_export} && {}", bundle_desc.version_check_cmd()
    )).await;
    if claw_ok.is_err() {
        anyhow::bail!("Bundle does not contain {} — invalid bundle", bundle_desc.display_name);
    }

    let oc_version = claw_ok.unwrap_or_default().trim().to_string();
    send(tx, &format!("{} {oc_version} ready (from bundle)", bundle_desc.display_name), 68, InstallStage::InstallOpenClaw).await;

    // Write platform-appropriate env file so the bundled PATH persists
    #[cfg(not(target_os = "windows"))]
    {
        let profile_path = install_dir.join("env.sh");
        let profile = format!(
            "export PATH=\"{}:{}:$PATH\"\n",
            node_bin.display(), modules_bin.display()
        );
        tokio::fs::write(&profile_path, &profile).await?;
    }
    #[cfg(target_os = "windows")]
    {
        let profile_path = install_dir.join("env.bat");
        let profile = format!(
            "@set PATH={};{};%PATH%\r\n",
            node_bin.display(), modules_bin.display()
        );
        tokio::fs::write(&profile_path, &profile).await?;
    }

    // ---- API Key ----
    if let Some(ref api_key) = opts.api_key {
        send(tx, "Storing API key...", 72, InstallStage::StoreApiKey).await;
        keychain::store_api_key(&opts.instance_name, api_key)?;
        if let Some(cmd) = bundle_desc.set_apikey_cmd(&shell_escape(api_key)) {
            backend.exec(&format!(
                "{path_export} && {cmd} 2>/dev/null || true"
            )).await?;
        }
    }

    // ---- Start gateway ----
    let gateway_cmd = bundle_desc.gateway_start_cmd(opts.gateway_port);
    send(tx, &format!("Starting {} gateway...", bundle_desc.display_name), 80, InstallStage::StartOpenClaw).await;
    #[cfg(not(target_os = "windows"))]
    backend.exec(&format!(
        "{path_export} && nohup {gateway_cmd} > /tmp/clawenv-gateway-{}.log 2>&1 &",
        opts.instance_name
    )).await?;
    #[cfg(target_os = "windows")]
    backend.exec(&format!(
        "{path_export} && start /b {gateway_cmd} > NUL 2>&1"
    )).await?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    send(tx, &format!("{} gateway started", bundle_desc.display_name), 85, InstallStage::StartOpenClaw).await;

    // ---- Save config ----
    send(tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    config.config_mut().instances.retain(|i| i.name != opts.instance_name);
    config.config_mut().instances.push(InstanceConfig {
        name: opts.instance_name.clone(),
        claw_type: opts.claw_type.clone(),
        version: oc_version,
        sandbox_type: SandboxType::Native,
        sandbox_id: format!("native-{}", opts.instance_name),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_upgraded_at: String::new(),
        gateway: GatewayConfig {
            gateway_port: opts.gateway_port,
            ttyd_port: 0,
            webchat_enabled: true,
            channels: Default::default(),
        },
        resources: ResourceConfig::default(),
        browser: Default::default(),
        cached_latest_version: String::new(),
        cached_version_check_at: String::new(),
    });
    config.save()?;

    send(tx, "Installation complete! (from bundle)", 100, InstallStage::Complete).await;
    Ok(())
}

// ---- Node.js detection ----

/// Get the ClawEnv-private Node.js directory (~/.clawenv/node/).
fn clawenv_node_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".clawenv/node")
}

/// Get the node binary path inside ClawEnv's private Node.js.
fn clawenv_node_bin() -> std::path::PathBuf {
    let dir = clawenv_node_dir();
    #[cfg(target_os = "windows")]
    { dir.join("node.exe") }
    #[cfg(not(target_os = "windows"))]
    { dir.join("bin/node") }
}

/// Ensure ClawEnv's private Node.js is in this process's PATH.
fn ensure_node_in_path() {
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

async fn has_node() -> bool {
    // Check ClawEnv's private Node.js first
    if clawenv_node_bin().exists() {
        ensure_node_in_path();
        return true;
    }
    // Fallback: check system PATH
    crate::platform::process::silent_cmd("node")
        .args(["--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn node_version() -> String {
    ensure_node_in_path();
    crate::platform::process::silent_cmd("node")
        .args(["--version"])
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

// ---- Platform-specific Node.js installation ----

#[cfg(target_os = "macos")]
async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    use tokio::process::Command;

    send(tx, "Downloading Node.js for macOS...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0"; // LTS
    let url = format!(
        "{nodejs_dist_base}/{version}/node-{version}-darwin-{arch}.pkg"
    );
    let pkg_path = "/tmp/clawenv-node.pkg";

    // Download
    let status = Command::new("curl")
        .args(["-fSL", "-o", pkg_path, &url])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to download Node.js from {url}");
    }

    send(tx, "Installing Node.js (may require admin password)...", 18, InstallStage::EnsurePrerequisites).await;

    // Install via macOS installer (triggers admin password dialog)
    let status = Command::new("sudo")
        .args(["installer", "-pkg", pkg_path, "-target", "/"])
        .status()
        .await?;

    // Cleanup
    tokio::fs::remove_file(pkg_path).await.ok();

    if !status.success() {
        anyhow::bail!("Node.js installation failed. Please install manually from https://nodejs.org");
    }

    // Verify
    if !has_node().await {
        anyhow::bail!("Node.js installed but not found in PATH. Please restart the application.");
    }

    Ok(())
}

#[cfg(target_os = "windows")]
async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    send(tx, "Downloading Node.js for Windows...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0";
    let url = format!(
        "{nodejs_dist_base}/{version}/node-{version}-win-{arch}.zip"
    );

    // Install to ~/.clawenv/node/ (no admin needed, self-contained)
    let node_dir = clawenv_node_dir();
    tokio::fs::create_dir_all(&node_dir).await?;

    let zip_path = node_dir.parent().unwrap_or(&node_dir).join("node.zip");

    // Download using curl (built into Windows 11)
    let status = crate::platform::process::silent_cmd("curl.exe")
        .args(["-fSL", "-o", &zip_path.to_string_lossy(), &url])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to download Node.js from {url}");
    }

    send(tx, "Extracting Node.js...", 18, InstallStage::EnsurePrerequisites).await;

    // Extract zip using PowerShell (no admin needed)
    let extract_cmd = format!(
        "Expand-Archive -Path '{}' -DestinationPath '{}' -Force; \
         $d = Get-ChildItem '{}' -Directory | Select-Object -First 1; \
         if ($d) {{ Get-ChildItem $d.FullName | Move-Item -Destination '{}' -Force }}",
        zip_path.to_string_lossy(),
        node_dir.to_string_lossy(),
        node_dir.to_string_lossy(),
        node_dir.to_string_lossy(),
    );
    let status = crate::platform::process::silent_cmd("powershell")
        .args(["-WindowStyle", "Hidden", "-Command", &extract_cmd])
        .status()
        .await?;

    // Cleanup zip
    tokio::fs::remove_file(&zip_path).await.ok();

    if !status.success() {
        anyhow::bail!("Failed to extract Node.js");
    }

    send(tx, "Node.js installed to ~/.clawenv/node", 22, InstallStage::EnsurePrerequisites).await;

    // Add to PATH for this process
    ensure_node_in_path();

    Ok(())
}

#[cfg(target_os = "linux")]
async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    use tokio::process::Command;

    send(tx, "Downloading Node.js for Linux...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0";
    let url = format!(
        "{nodejs_dist_base}/{version}/node-{version}-linux-{arch}.tar.xz"
    );

    // Install to ~/.clawenv/node (no root needed)
    let node_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?
        .join(".clawenv/node");
    tokio::fs::create_dir_all(&node_dir).await?;

    let status = Command::new("sh")
        .args(["-c", &format!(
            "curl -fSL '{url}' | tar -xJ --strip-components=1 -C '{}'",
            node_dir.display()
        )])
        .status()
        .await?;

    if !status.success() {
        anyhow::bail!("Failed to download and extract Node.js");
    }

    send(tx, "Node.js installed to ~/.clawenv/node", 22, InstallStage::EnsurePrerequisites).await;

    // Add to PATH for this process and hint for future
    let bin_path = node_dir.join("bin");
    let current_path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{current_path}", bin_path.display()));

    if !has_node().await {
        anyhow::bail!(
            "Node.js installed to {} but not reachable. Add to PATH: export PATH=\"{}:$PATH\"",
            node_dir.display(), bin_path.display()
        );
    }

    Ok(())
}
