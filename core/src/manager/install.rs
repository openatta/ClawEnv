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
    pub install_mcp_bridge: bool,
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
            install_mcp_bridge: true, // default ON
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
            tracing::error!("Installation failed: {e}");
            // Do NOT destroy the VM — user can retry from the failed step
            send(&tx, &format!("Installation failed: {e}"), 0, InstallStage::Failed).await;
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

    let sandbox_type = if opts.use_native { SandboxType::Native } else { SandboxType::from_os() };

    provision_sandbox(opts, backend, &sandbox_opts, tx).await?;
    install_dependencies(opts, backend, config, sandbox_type, tx).await?;
    configure_instance(opts, backend, tx).await?;
    save_instance_config(opts, config, backend, sandbox_type, tx).await?;

    Ok(())
}

/// VM creation + boot verification
async fn provision_sandbox(
    _opts: &InstallOptions,
    backend: &dyn SandboxBackend,
    sandbox_opts: &SandboxOpts,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    // --- Step: Create VM (skip if already exists) ---
    let vm_exists = backend.exec("echo ok").await.map(|o| o.contains("ok")).unwrap_or(false);

    if vm_exists {
        send(tx, "VM already exists, resuming from current state...", 30, InstallStage::CreateVm).await;
    } else {
        send(tx, "Creating virtual machine...", 15, InstallStage::CreateVm).await;

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
        backend.create(sandbox_opts).await?;
        heartbeat.abort();
    }

    // --- Step: Boot VM (verify accessible) ---
    send(tx, "Verifying VM is accessible...", 30, InstallStage::BootVm).await;
    let vm_check = backend.exec("echo ok").await;
    match vm_check {
        Ok(out) if out.contains("ok") => {
            send(tx, "VM is ready", 35, InstallStage::BootVm).await;
        }
        _ => {
            // Try starting it first (it may be stopped)
            send(tx, "VM not responding, attempting to start...", 32, InstallStage::BootVm).await;
            backend.start().await?;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let retry = backend.exec("echo ok").await;
            if retry.map(|o| o.contains("ok")).unwrap_or(false) {
                send(tx, "VM started successfully", 35, InstallStage::BootVm).await;
            } else {
                anyhow::bail!("VM not accessible. Check Lima status with: limactl list");
            }
        }
    }

    // Log workspace mount info (Lima's template:alpine mounts home directory by default)
    send(tx, "Workspace directory mounted at /Users/konghan/.clawenv/workspaces/default", 37, InstallStage::BootVm).await;

    Ok(())
}

/// Proxy config + apk add + node check + openclaw install
async fn install_dependencies(
    opts: &InstallOptions,
    backend: &dyn SandboxBackend,
    config: &mut ConfigManager,
    sandbox_type: SandboxType,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
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
    // Podman: packages are pre-installed in Containerfile, skip provisioning
    // Lima/WSL2/Native: need to install packages after VM creation
    let is_podman = sandbox_type == SandboxType::PodmanAlpine;
    let proxy_prefix = if proxy_config.enabled && !proxy_config.http_proxy.is_empty() {
        ". /etc/profile.d/proxy.sh 2>/dev/null; "
    } else {
        ""
    };

    if is_podman {
        send(tx, "Dependencies pre-installed in container image", 50, InstallStage::InstallDeps).await;
    } else {
        send(tx, "Installing dependencies (git, curl, bash)...", 42, InstallStage::InstallDeps).await;
        backend.exec(&format!("{proxy_prefix}sudo apk add --no-cache git curl bash 2>&1 || apk add --no-cache git curl bash 2>&1")).await?;

        send(tx, "Checking Node.js availability...", 48, InstallStage::InstallDeps).await;
        let has_node = backend.exec("which node 2>/dev/null").await.unwrap_or_default();
        if has_node.trim().is_empty() {
            send(tx, "Installing Node.js + npm...", 50, InstallStage::InstallDeps).await;
            backend.exec(&format!("{proxy_prefix}sudo apk add --no-cache nodejs npm 2>&1 || apk add --no-cache nodejs npm 2>&1")).await?;
        } else {
            send(tx, "Node.js already available", 50, InstallStage::InstallDeps).await;
        }
    }

    // --- Step: Install OpenClaw ---
    let installed = backend.exec("openclaw --version 2>/dev/null || echo ''").await.unwrap_or_default();
    if is_podman && !installed.trim().is_empty() {
        send(tx, &format!("OpenClaw pre-installed: {}", installed.trim()), 65, InstallStage::InstallOpenClaw).await;
    } else if installed.trim().is_empty() {
        send(tx, "Installing OpenClaw (this may take 1-2 minutes)...", 55, InstallStage::InstallOpenClaw).await;
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

    Ok(())
}

/// API key storage + browser install + gateway start
async fn configure_instance(
    opts: &InstallOptions,
    backend: &dyn SandboxBackend,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
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
        backend.exec(
            "sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont 2>&1"
        ).await?;
        send(tx, "Browser components installed", 80, InstallStage::InstallBrowser).await;
    }

    // --- Step: Install terminal services (ttyd + openssh) ---
    send(tx, "Installing terminal services...", 81, InstallStage::StartOpenClaw).await;
    backend.exec("sudo apk add --no-cache ttyd openssh 2>&1 || apk add --no-cache ttyd openssh 2>&1").await?;
    // Generate SSH host keys if not present
    backend.exec("sudo ssh-keygen -A 2>/dev/null || true").await?;
    // Start ttyd on port 7681 (WebSocket terminal)
    let ttyd_port = opts.gateway_port + 4681; // e.g. 3000 -> 7681
    backend.exec(&format!(
        "nohup ttyd -p {ttyd_port} -W /bin/sh > /tmp/ttyd.log 2>&1 &"
    )).await?;
    send(tx, "Terminal services ready", 83, InstallStage::StartOpenClaw).await;

    // --- Step: Install MCP Bridge Plugin ---
    if opts.install_mcp_bridge {
        send(tx, "Installing MCP Bridge plugin...", 82, InstallStage::StartOpenClaw).await;
        // Embed the MCP bridge JS at compile time
        let mcp_js = include_str!("../../../assets/mcp/mcp-bridge.mjs");
        // Write to workspace directory in sandbox
        backend.exec("mkdir -p /workspace/mcp-bridge 2>/dev/null || mkdir -p ~/mcp-bridge").await?;
        // Use heredoc to write the file content
        let escaped = mcp_js.replace('\'', "'\\''");
        backend.exec(&format!(
            "cat > ~/mcp-bridge/index.mjs << 'MCPEOF'\n{mcp_js}\nMCPEOF"
        )).await?;
        // Register with OpenClaw MCP config
        let gateway_token = backend.exec(
            "cat ~/.openclaw/openclaw.json 2>/dev/null | grep -o '\"token\":[ ]*\"[^\"]*\"' | head -1 | sed 's/.*\"\\([^\"]*\\)\"/\\1/'"
        ).await.unwrap_or_default();
        let token = gateway_token.trim();
        if !token.is_empty() {
            backend.exec(&format!(
                "OPENCLAW_GATEWAY_URL=ws://127.0.0.1:{port} OPENCLAW_GATEWAY_TOKEN={token} openclaw mcp set clawenv '{{\"command\":\"node\",\"args\":[\"/home/clawenv/mcp-bridge/index.mjs\"]}}' 2>/dev/null || true",
                port = opts.gateway_port, token = token
            )).await?;
        }
        send(tx, "MCP Bridge plugin installed", 84, InstallStage::StartOpenClaw).await;
    }

    // --- Step: Start OpenClaw ---
    send(tx, "Starting OpenClaw daemon...", 85, InstallStage::StartOpenClaw).await;
    let port = opts.gateway_port;
    backend.exec(&format!("nohup openclaw gateway --port {port} --allow-unconfigured > /tmp/openclaw-gateway.log 2>&1 &")).await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    send(tx, "OpenClaw started", 88, InstallStage::StartOpenClaw).await;

    Ok(())
}

/// Config persistence
async fn save_instance_config(
    opts: &InstallOptions,
    config: &mut ConfigManager,
    backend: &dyn SandboxBackend,
    sandbox_type: SandboxType,
    tx: &mpsc::Sender<InstallProgress>,
) -> Result<()> {
    // --- Step: Save Config ---
    send(tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    let version = backend.exec("openclaw --version 2>/dev/null || echo unknown").await
        .unwrap_or_else(|_| opts.claw_version.clone());

    // Remove any existing instance with same name (idempotent for retry)
    config.config_mut().instances.retain(|i| i.name != opts.instance_name);

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
            ttyd_port: opts.gateway_port + 4681,
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
