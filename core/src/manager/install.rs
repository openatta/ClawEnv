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

/// Validate instance name — only allow safe characters
pub fn validate_instance_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        anyhow::bail!("Instance name must be 1-63 characters");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!(
            "Instance name can only contain alphanumeric characters, hyphens, and underscores"
        );
    }
    if name.starts_with('-') {
        anyhow::bail!("Instance name cannot start with a hyphen");
    }
    Ok(())
}

/// Installation options from the wizard
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

/// Progress event emitted during installation
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
    CreateSandbox,
    ConfigureProxy,
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
    // Validate instance name before anything else
    validate_instance_name(&opts.instance_name)?;

    // 1. Select backend
    send(&tx, "Detecting platform...", 5, InstallStage::DetectPlatform).await;
    let backend: Box<dyn SandboxBackend> = if opts.use_native {
        Box::new(native_backend(&opts.instance_name))
    } else {
        detect_backend()?
    };
    tracing::info!("Using backend: {}", backend.name());

    // 2. Ensure prerequisites
    send(
        &tx,
        &format!("Checking {} prerequisites...", backend.name()),
        10,
        InstallStage::EnsurePrerequisites,
    )
    .await;
    backend.ensure_prerequisites().await?;

    // Run the actual install steps with rollback on failure
    match do_install(&opts, config, backend.as_ref(), &tx).await {
        Ok(()) => {
            send(&tx, "Installation complete!", 100, InstallStage::Complete).await;
            Ok(())
        }
        Err(e) => {
            tracing::error!("Installation failed, rolling back: {e}");
            send(
                &tx,
                &format!("Installation failed: {e}. Cleaning up..."),
                0,
                InstallStage::Failed,
            )
            .await;
            // Best-effort cleanup — destroy the sandbox
            if let Err(cleanup_err) = backend.destroy().await {
                tracing::warn!("Rollback cleanup failed: {cleanup_err}");
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
    // 3. Create sandbox
    let sandbox_opts = SandboxOpts {
        instance_name: opts.instance_name.clone(),
        claw_version: opts.claw_version.clone(),
        alpine_version: "latest-stable".into(),
        memory_mb: 512,
        cpu_cores: 2,
        install_browser: opts.install_browser,
        install_mode: opts.install_mode.clone(),
    };

    send(tx, "Creating sandbox VM (this may take a few minutes)...", 20, InstallStage::CreateSandbox).await;

    // Heartbeat — keeps UI alive during long VM creation
    let tx_hb = tx.clone();
    let heartbeat = tokio::spawn(async move {
        let mut tick = 0u32;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            tick += 1;
            let pct = std::cmp::min(20 + tick as u8 * 2, 44);
            let msg = match tick {
                1..=2 => "Downloading Alpine Linux image...",
                3..=4 => "Booting virtual machine...",
                5..=8 => "Waiting for VM to become ready...",
                _ => "Still working, please wait...",
            };
            send(&tx_hb, msg, pct, InstallStage::CreateSandbox).await;
        }
    });

    let create_result = backend.create(&sandbox_opts).await;
    heartbeat.abort();
    create_result?;

    send(tx, "Sandbox VM ready", 45, InstallStage::CreateSandbox).await;

    // 4. Apply proxy if configured
    let proxy_config = &config.config().clawenv.proxy;
    if proxy_config.enabled {
        send(tx, "Configuring proxy...", 50, InstallStage::ConfigureProxy).await;
        proxy::apply_proxy(backend, proxy_config).await?;
    }

    // 5. Store API key in keychain and inject into sandbox (safely escaped)
    if let Some(ref api_key) = opts.api_key {
        send(tx, "Storing API key securely...", 60, InstallStage::StoreApiKey).await;
        keychain::store_api_key(&opts.instance_name, api_key)?;
        let escaped = shell_escape(api_key);
        backend
            .exec(&format!("openclaw config set apiKey '{escaped}'"))
            .await?;
    }

    // 6. Optional: install browser
    if opts.install_browser {
        send(tx, "Installing Chromium + noVNC...", 70, InstallStage::InstallBrowser).await;
        backend
            .exec("apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont")
            .await?;
    }

    // 7. Start OpenClaw
    send(tx, "Starting OpenClaw...", 80, InstallStage::StartOpenClaw).await;
    backend.start().await?;
    backend.exec("openclaw start --daemon").await?;

    // 8. Verify OpenClaw actually started
    let health = crate::monitor::InstanceMonitor::check_health(backend).await;
    if health == crate::monitor::InstanceHealth::Unreachable {
        // Give it a few more seconds
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let health2 = crate::monitor::InstanceMonitor::check_health(backend).await;
        if health2 == crate::monitor::InstanceHealth::Unreachable {
            anyhow::bail!("OpenClaw failed to start after installation");
        }
    }

    // 9. Save config — only after everything succeeds
    send(tx, "Saving configuration...", 90, InstallStage::SaveConfig).await;
    let sandbox_type = if opts.use_native {
        SandboxType::Native
    } else {
        SandboxType::from_os()
    };

    let version = backend
        .exec("openclaw --version")
        .await
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

    Ok(())
}

async fn send(tx: &mpsc::Sender<InstallProgress>, message: &str, percent: u8, stage: InstallStage) {
    let _ = tx.send(InstallProgress {
        message: message.to_string(),
        percent,
        stage,
    })
    .await;
}
