use anyhow::{anyhow, Result};
use tokio::sync::mpsc;

use crate::config::{keychain, proxy, ConfigManager, InstanceConfig, OpenClawConfig, ResourceConfig};
use crate::sandbox::{
    detect_backend, native_backend, InstallMode, SandboxBackend, SandboxOpts, SandboxType,
};

/// Escape a string for safe use inside single-quoted shell arguments
pub fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Validate instance name
pub fn validate_instance_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        anyhow::bail!("Instance name must be 1-63 characters");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("Instance name can only contain alphanumeric characters, hyphens, and underscores");
    }
    if name.starts_with('-') {
        anyhow::bail!("Instance name cannot start with a hyphen");
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub instance_name: String,
    pub claw_version: String,
    pub install_mode: InstallMode,
    pub install_browser: bool,
    pub api_key: Option<String>,
    pub use_native: bool,
    pub gateway_port: u16,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            instance_name: "default".into(),
            claw_version: "latest".into(),
            install_mode: InstallMode::OnlineBuild,
            install_browser: false,
            api_key: None,
            use_native: false,
            gateway_port: 3000,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstallProgress {
    pub message: String,
    pub percent: u8,
    pub stage: InstallStage,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStage {
    DetectPlatform,
    EnsurePrerequisites,
    CreateVm,
    BootVm,
    ConfigureProxy,
    InstallDeps,
    InstallOpenClaw,
    StoreApiKey,
    InstallBrowser,
    StartOpenClaw,
    SaveConfig,
    Complete,
    Failed,
}

/// Run the full installation flow with rollback on failure
pub async fn install(
    opts: InstallOptions,
    config: &mut ConfigManager,
    tx: mpsc::Sender<InstallProgress>,
) -> Result<()> {
    validate_instance_name(&opts.instance_name)?;

    send(&tx, "Detecting platform...", 5, InstallStage::DetectPlatform).await;
    let backend: Box<dyn SandboxBackend> = if opts.use_native {
        Box::new(native_backend(&opts.instance_name))
    } else {
        detect_backend()?
    };
    tracing::info!("Using backend: {}", backend.name());

    send(&tx, &format!("Checking {} prerequisites...", backend.name()), 8, InstallStage::EnsurePrerequisites).await;
    backend.ensure_prerequisites().await?;
    send(&tx, &format!("{} ready", backend.name()), 10, InstallStage::EnsurePrerequisites).await;

    // Run install with rollback on failure
    match do_install(&opts, config, backend.as_ref(), &tx).await {
        Ok(()) => {
            send(&tx, "Installation complete!", 100, InstallStage::Complete).await;
            Ok(())
        }
        Err(e) => {
            tracing::error!("Installation failed, rolling back: {e}");
            send(&tx, &format!("Installation failed: {e}. Cleaning up..."), 0, InstallStage::Failed).await;
            if let Err(ce) = backend.destroy().await {
                tracing::warn!("Rollback cleanup failed: {ce}");
            }
            Err(e)
        }
    }
}

async fn do_install(
    opts: &InstallOptions,
    config: &mut ConfigManager,
    backend: &dyn SandboxBackend,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    let sandbox_opts = SandboxOpts {
        instance_name: opts.instance_name.clone(),
        claw_version: opts.claw_version.clone(),
        alpine_version: "latest-stable".into(),
        memory_mb: 512,
        cpu_cores: 2,
        install_browser: opts.install_browser,
        install_mode: opts.install_mode.clone(),
    };

    // --- Step: Create VM ---
    send(tx, "Creating virtual machine...", 15, InstallStage::CreateVm).await;

    // Heartbeat during long VM creation
    let tx_hb = tx.clone();
    let heartbeat = tokio::spawn(async move {
        let mut tick = 0u32;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            tick += 1;
            let pct = std::cmp::min(15 + tick as u8 * 2, 29);
            let msg = match tick {
                1..=2 => "Downloading VM image...",
                3..=5 => "Booting virtual machine...",
                _ => "Waiting for VM to become ready...",
            };
            send(&tx_hb, msg, pct, InstallStage::CreateVm).await;
        }
    });
    backend.create(&sandbox_opts).await?;
    heartbeat.abort();

    // --- Step: Boot VM (verify it's accessible) ---
    send(tx, "Verifying VM is accessible...", 30, InstallStage::BootVm).await;
    let vm_check = backend.exec("echo ok 2>&1").await;
    match vm_check {
        Ok(out) if out.contains("ok") => {
            send(tx, "VM is ready", 35, InstallStage::BootVm).await;
        }
        _ => {
            anyhow::bail!("VM created but not accessible via exec");
        }
    }

    // --- Step: Configure Proxy (BEFORE installing packages!) ---
    let proxy_config = &config.config().clawenv.proxy;
    if proxy_config.enabled && !proxy_config.http_proxy.is_empty() {
        send(tx, "Configuring proxy inside sandbox...", 38, InstallStage::ConfigureProxy).await;
        proxy::apply_proxy(backend, proxy_config).await?;
        send(tx, "Proxy configured", 40, InstallStage::ConfigureProxy).await;
    } else {
        send(tx, "No proxy configured, using direct connection", 40, InstallStage::ConfigureProxy).await;
    }

    // --- Step: Install Dependencies ---
    send(tx, "Installing dependencies (git, curl, bash)...", 42, InstallStage::InstallDeps).await;
    // Source proxy env if it was set
    let proxy_prefix = if proxy_config.enabled && !proxy_config.http_proxy.is_empty() {
        ". /etc/profile.d/proxy.sh 2>/dev/null; "
    } else {
        ""
    };
    backend.exec(&format!("{proxy_prefix}sudo apk add --no-cache git curl bash 2>&1 || apk add --no-cache git curl bash 2>&1")).await?;

    // Check Node.js
    send(tx, "Checking Node.js availability...", 48, InstallStage::InstallDeps).await;
    let has_node = backend.exec("which node 2>/dev/null").await.unwrap_or_default();
    if has_node.trim().is_empty() {
        send(tx, "Installing Node.js + npm...", 50, InstallStage::InstallDeps).await;
        backend.exec(&format!("{proxy_prefix}sudo apk add --no-cache nodejs npm 2>&1 || apk add --no-cache nodejs npm 2>&1")).await?;
    } else {
        send(tx, "Node.js already available", 50, InstallStage::InstallDeps).await;
    }

    // --- Step: Install OpenClaw ---
    send(tx, "Installing OpenClaw (this may take 1-2 minutes)...", 55, InstallStage::InstallOpenClaw).await;
    let installed = backend.exec("openclaw --version 2>/dev/null || echo ''").await.unwrap_or_default();
    if installed.trim().is_empty() {
        // Use exec_with_progress for streaming npm output
        let (progress_tx, mut progress_rx) = mpsc::channel::<String>(64);

        // Forward npm output lines to install progress
        let tx_npm = tx.clone();
        let npm_log = tokio::spawn(async move {
            while let Some(line) = progress_rx.recv().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    send(&tx_npm, &format!("  npm: {}", &trimmed[..trimmed.len().min(80)]), 58, InstallStage::InstallOpenClaw).await;
                }
            }
        });

        let install_cmd = format!(
            "{proxy_prefix}sudo npm install -g openclaw@{ver} 2>&1 || npm install -g openclaw@{ver} 2>&1",
            ver = opts.claw_version
        );
        let install_result = backend.exec_with_progress(&install_cmd, &progress_tx).await;
        drop(progress_tx);
        npm_log.await.ok();

        install_result?;
        send(tx, "OpenClaw installed successfully", 65, InstallStage::InstallOpenClaw).await;
    } else {
        send(tx, &format!("OpenClaw already installed: {}", installed.trim()), 65, InstallStage::InstallOpenClaw).await;
    }

    // --- Step: Store API Key ---
    if let Some(ref api_key) = opts.api_key {
        send(tx, "Storing API key in keychain...", 70, InstallStage::StoreApiKey).await;
        keychain::store_api_key(&opts.instance_name, api_key)?;
        let escaped = shell_escape(api_key);
        backend.exec(&format!("openclaw config set apiKey '{escaped}' 2>/dev/null || true")).await?;
        send(tx, "API key stored", 73, InstallStage::StoreApiKey).await;
    }

    // --- Step: Install Browser (optional) ---
    if opts.install_browser {
        send(tx, "Installing Chromium + noVNC...", 75, InstallStage::InstallBrowser).await;
        backend.exec(&format!(
            "{proxy_prefix}sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont 2>&1"
        )).await?;
        send(tx, "Browser components installed", 80, InstallStage::InstallBrowser).await;
    }

    // --- Step: Start OpenClaw ---
    send(tx, "Starting OpenClaw daemon...", 85, InstallStage::StartOpenClaw).await;
    backend.exec("openclaw start --daemon 2>/dev/null || true").await?;
    send(tx, "OpenClaw started", 88, InstallStage::StartOpenClaw).await;

    // --- Step: Save Config ---
    send(tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    let sandbox_type = if opts.use_native { SandboxType::Native } else { SandboxType::from_os() };
    let version = backend.exec("openclaw --version 2>/dev/null || echo unknown").await
        .unwrap_or_else(|_| opts.claw_version.clone());

    config.config_mut().instances.push(InstanceConfig {
        name: opts.instance_name.clone(),
        claw_type: "openclaw".into(),
        version: version.trim().to_string(),
        sandbox_type,
        sandbox_id: format!("clawenv-{}", opts.instance_name),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_upgraded_at: String::new(),
        openclaw: OpenClawConfig {
            gateway_port: opts.gateway_port,
            webchat_enabled: true,
            channels: Default::default(),
        },
        resources: ResourceConfig::default(),
        browser: Default::default(),
    });
    config.save()?;
    send(tx, "Configuration saved", 95, InstallStage::SaveConfig).await;

    Ok(())
}

async fn send(tx: &mpsc::Sender<InstallProgress>, message: &str, percent: u8, stage: InstallStage) {
    let _ = tx.send(InstallProgress {
        message: message.to_string(),
        percent,
        stage,
    }).await;
}
