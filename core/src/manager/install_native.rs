//! Native install path — no VM, direct host installation.
//!
//! Platform-specific Node.js installation:
//!   macOS:   Download .pkg from nodejs.org, `installer` command (needs admin password)
//!   Windows: Download .msi, `msiexec /i /qn` (needs UAC)
//!   Linux:   Download tar.xz to ~/.clawenv/node/, add to PATH

use anyhow::Result;
use tokio::sync::mpsc;

use crate::config::{keychain, ConfigManager, InstanceConfig, OpenClawConfig, ResourceConfig};
use crate::sandbox::{SandboxType, native_backend, SandboxBackend};

use super::install::{InstallOptions, InstallProgress, InstallStage, send, shell_escape};

/// Native install flow — no VM, no MCP Bridge, no ttyd.
pub async fn install_native(
    opts: &InstallOptions,
    config: &mut ConfigManager,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    // ---- Step 1: Ensure Node.js + npm ----
    send(tx, "Checking Node.js environment...", 10, InstallStage::EnsurePrerequisites).await;

    if !has_node().await {
        send(tx, "Node.js not found, installing...", 12, InstallStage::EnsurePrerequisites).await;
        install_nodejs(tx).await?;
        send(tx, "Node.js installed", 25, InstallStage::EnsurePrerequisites).await;
    } else {
        let ver = node_version().await;
        send(tx, &format!("Node.js {ver} ready"), 25, InstallStage::EnsurePrerequisites).await;
    }

    // ---- Step 2: Install OpenClaw ----
    let backend = native_backend(&opts.instance_name);

    let oc_installed = backend.exec("openclaw --version 2>/dev/null || echo ''").await
        .map(|o| !o.trim().is_empty()).unwrap_or(false);

    if !oc_installed {
        send(tx, "Installing OpenClaw...", 30, InstallStage::InstallOpenClaw).await;

        // Ensure install dir exists before exec (NativeBackend uses it as cwd)
        let install_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".clawenv/native")
            .join(&opts.instance_name);
        tokio::fs::create_dir_all(&install_dir).await?;

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
            &format!("npm install -g --loglevel verbose openclaw@{}", opts.claw_version),
            &progress_tx,
        ).await;
        drop(progress_tx);
        ui_task.await.ok();
        result?;

        send(tx, "OpenClaw installed", 68, InstallStage::InstallOpenClaw).await;
    } else {
        send(tx, "OpenClaw already installed", 68, InstallStage::InstallOpenClaw).await;
    }

    let oc_version = backend.exec("openclaw --version 2>/dev/null || echo unknown").await.unwrap_or_default();

    // ---- Step 3: API Key ----
    if let Some(ref api_key) = opts.api_key {
        send(tx, "Storing API key...", 72, InstallStage::StoreApiKey).await;
        keychain::store_api_key(&opts.instance_name, api_key)?;
        let esc = shell_escape(api_key);
        backend.exec(&format!("openclaw config set apiKey '{esc}' 2>/dev/null || true")).await?;
    }

    // ---- Step 4: Start gateway ----
    send(tx, "Starting OpenClaw gateway...", 80, InstallStage::StartOpenClaw).await;
    let port = opts.gateway_port;
    backend.exec(&format!(
        "nohup openclaw gateway --port {port} --allow-unconfigured > /tmp/openclaw-gateway-{}.log 2>&1 &",
        opts.instance_name
    )).await?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    send(tx, "OpenClaw gateway started", 85, InstallStage::StartOpenClaw).await;

    // ---- Step 5: Save config ----
    // Native: no MCP Bridge, no ttyd, no host IP sync
    send(tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    config.config_mut().instances.retain(|i| i.name != opts.instance_name);
    config.config_mut().instances.push(InstanceConfig {
        name: opts.instance_name.clone(),
        claw_type: "openclaw".into(),
        version: oc_version.trim().to_string(),
        sandbox_type: SandboxType::Native,
        sandbox_id: format!("native-{}", opts.instance_name),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_upgraded_at: String::new(),
        openclaw: OpenClawConfig {
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

// ---- Node.js detection ----

async fn has_node() -> bool {
    tokio::process::Command::new("node")
        .args(["--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn node_version() -> String {
    tokio::process::Command::new("node")
        .args(["--version"])
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

// ---- Platform-specific Node.js installation ----

#[cfg(target_os = "macos")]
async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>) -> Result<()> {
    use tokio::process::Command;

    send(tx, "Downloading Node.js for macOS...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0"; // LTS
    let url = format!(
        "https://nodejs.org/dist/{version}/node-{version}-darwin-{arch}.pkg"
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
async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>) -> Result<()> {
    use tokio::process::Command;

    send(tx, "Downloading Node.js for Windows...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0";
    let url = format!(
        "https://nodejs.org/dist/{version}/node-{version}-{arch}.msi"
    );

    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Public".into());
    let msi_path = format!("{home}\\clawenv-node.msi");

    // Download via PowerShell
    let status = Command::new("powershell")
        .args(["-Command", &format!(
            "Invoke-WebRequest -Uri '{url}' -OutFile '{msi_path}'"
        )])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to download Node.js");
    }

    send(tx, "Installing Node.js (may require admin approval)...", 18, InstallStage::EnsurePrerequisites).await;

    // Silent install via msiexec (triggers UAC)
    let status = Command::new("msiexec")
        .args(["/i", &msi_path, "/qn", "/norestart"])
        .status()
        .await?;

    // Cleanup
    tokio::fs::remove_file(&msi_path).await.ok();

    if !status.success() {
        anyhow::bail!("Node.js installation failed. Please install from https://nodejs.org");
    }

    Ok(())
}

#[cfg(target_os = "linux")]
async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>) -> Result<()> {
    use tokio::process::Command;

    send(tx, "Downloading Node.js for Linux...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0";
    let url = format!(
        "https://nodejs.org/dist/{version}/node-{version}-linux-{arch}.tar.xz"
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
