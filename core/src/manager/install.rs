use anyhow::Result;
use tokio::sync::mpsc;

use crate::claw::ClawRegistry;
use crate::config::{keychain, mirrors, ConfigManager, InstanceConfig, GatewayConfig, ResourceConfig, BrowserConfig};
use crate::platform::network;
use crate::sandbox::{
    detect_backend_for, InstallMode, SandboxBackend, SandboxOpts, SandboxType,
};

/// Escape a string for use inside single-quoted shell arguments.
/// Use `crate::platform::shell_quote()` for the full-wrapped version.
pub fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Check that the gateway port is not already in use by another instance.
pub fn validate_port_available(config: &ConfigManager, instance_name: &str, port: u16) -> Result<()> {
    for inst in config.instances() {
        if inst.name != instance_name && inst.gateway.gateway_port == port {
            anyhow::bail!(
                "Port {port} is already used by instance '{}'. Choose a different port with --port.",
                inst.name
            );
        }
    }
    Ok(())
}

/// Check if a port is available (not bound by any process on localhost).
fn is_port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Find the next available gateway port starting from `base_port`, step 20.
/// Each instance reserves a 20-port block: base+0 gateway, +1 ttyd, +2 bridge, +3 cdp, +4 vnc.
pub fn next_available_port(config: &ConfigManager, base_port: u16) -> u16 {
    let used: std::collections::HashSet<u16> = config.instances().iter()
        .map(|i| i.gateway.gateway_port)
        .collect();
    let mut port = base_port;
    while used.contains(&port) {
        port = port.saturating_add(20);
        if port > 60000 { break; }
    }
    port
}

/// Allocate a specific sub-port within an instance's block.
/// Tries `base + offset` first; if occupied by another process, increments until free.
/// Stays within the 20-port block (base..base+19).
pub fn allocate_port(base: u16, offset: u16) -> u16 {
    let mut port = base + offset;
    let limit = base + 19;
    while port <= limit {
        if is_port_free(port) { return port; }
        port += 1;
    }
    // Fallback: return the original offset port even if occupied
    base + offset
}

pub fn validate_instance_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 63 {
        anyhow::bail!("Instance name must be 1-63 characters");
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("Instance name can only contain alphanumeric, hyphens, underscores");
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub instance_name: String,
    /// Claw type ID — key into ClawRegistry (e.g., "openclaw", "zeroclaw")
    pub claw_type: String,
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
            claw_type: "openclaw".into(),
            claw_version: "latest".into(),
            install_mode: InstallMode::OnlineBuild,
            install_browser: false,
            install_mcp_bridge: true,
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

/// Main install flow:
///   1. Detect platform, install Lima if needed
///   2. Create VM with provision (system packages only, ~1 min)
///   3. Run npm install openclaw as background script in VM, poll progress
///   4. Lightweight post-install config (API key, MCP bridge, start services)
pub async fn install(
    opts: InstallOptions,
    config: &mut ConfigManager,
    tx: mpsc::Sender<InstallProgress>,
) -> Result<()> {
    validate_instance_name(&opts.instance_name)?;
    validate_port_available(config, &opts.instance_name, opts.gateway_port)?;

    // Native mode: only one instance allowed
    if opts.use_native || matches!(opts.install_mode, InstallMode::NativeBundle { .. }) {
        let has_native = config.instances().iter().any(|i| {
            i.sandbox_type == SandboxType::Native && i.name != opts.instance_name
        });
        if has_native {
            anyhow::bail!(
                "A native instance already exists. Only one native instance is allowed. \
                 Use sandbox mode to create additional instances."
            );
        }
    }

    // Dispatch: Native vs Sandbox
    // NativeBundle always goes through native path regardless of use_native flag
    if opts.use_native || matches!(opts.install_mode, InstallMode::NativeBundle { .. }) {
        return super::install_native::install_native(&opts, config, &tx).await;
    }

    // ---- Sandbox path below ----
    // Resolve the claw descriptor for this install
    let registry = ClawRegistry::load();
    let desc = registry.get_strict(&opts.claw_type)?;

    send(&tx, "Detecting platform...", 5, InstallStage::DetectPlatform).await;
    let backend: Box<dyn SandboxBackend> = detect_backend_for(&opts.instance_name)?;

    send(&tx, &format!("Checking {} prerequisites...", backend.name()), 8, InstallStage::EnsurePrerequisites).await;
    backend.ensure_prerequisites().await?;
    send(&tx, &format!("{} ready", backend.name()), 10, InstallStage::EnsurePrerequisites).await;

    let sandbox_type = if opts.use_native { SandboxType::Native } else { SandboxType::from_os() };
    let mirrors_config = config.config().clawenv.mirrors.clone();

    // ---- Step 2: Create VM (provision = system packages only) ----
    // Check if VM exists AND has basic packages (node/npm). A VM that exists but
    // wasn't fully provisioned (e.g., interrupted install) is treated as non-existent.
    let vm_ready = match backend.exec("node --version 2>/dev/null").await {
        Ok(o) => o.trim().starts_with('v'),
        Err(_) => false,
    };

    if !vm_ready {
        send(&tx, "Creating VM (installing system packages)...", 12, InstallStage::CreateVm).await;

        let proxy_config = &config.config().clawenv.proxy;

        let mut provision_preamble = String::new();

        // Proxy exports
        if proxy_config.enabled && !proxy_config.http_proxy.is_empty() {
            let https = if proxy_config.https_proxy.is_empty() { &proxy_config.http_proxy } else { &proxy_config.https_proxy };
            provision_preamble.push_str(&format!(
                "export http_proxy=\"{}\" https_proxy=\"{}\" HTTP_PROXY=\"{}\" HTTPS_PROXY=\"{}\" no_proxy=\"localhost,127.0.0.1\"\n",
                proxy_config.http_proxy, https, proxy_config.http_proxy, https
            ));
        }

        // Mirror sources (Alpine APK + npm registry)
        provision_preamble.push_str(&mirrors::alpine_repo_script(&mirrors_config, "latest-stable"));
        provision_preamble.push_str(&mirrors::npm_registry_script(&mirrors_config));

        let proxy_script = if provision_preamble.trim().is_empty() {
            "# No proxy / mirrors".to_string()
        } else {
            provision_preamble
        };

        let alpine_mirror = if mirrors_config.is_default() {
            String::new()
        } else {
            mirrors_config.alpine_repo_url().to_string()
        };
        let npm_registry = if mirrors_config.is_default() {
            String::new()
        } else {
            mirrors_config.npm_registry_url().to_string()
        };

        let sandbox_opts = SandboxOpts {
            instance_name: opts.instance_name.clone(),
            claw_type: opts.claw_type.clone(),
            claw_version: opts.claw_version.clone(),
            alpine_version: "latest-stable".into(),
            memory_mb: 512,
            cpu_cores: 2,
            install_browser: opts.install_browser,
            install_mode: opts.install_mode.clone(),
            proxy_script,
            gateway_port: opts.gateway_port,
            alpine_mirror,
            npm_registry,
        };

        // Heartbeat while VM creates
        let tx_hb = tx.clone();
        let heartbeat = tokio::spawn(async move {
            let mut tick = 0u32;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                tick += 1;
                let pct = std::cmp::min(12 + tick as u8 * 2, 35);
                let msg = match tick {
                    1..=3 => "Downloading VM image...",
                    4..=8 => "Booting and provisioning...",
                    _ => "Installing system packages...",
                };
                send(&tx_hb, msg, pct, InstallStage::CreateVm).await;
            }
        });

        // 30-minute absolute timeout for VM creation (download + provision).
        // The heartbeat above provides activity feedback; this is a hard safety net.
        match tokio::time::timeout(
            std::time::Duration::from_secs(30 * 60),
            backend.create(&sandbox_opts),
        ).await {
            Ok(result) => result?,
            Err(_) => {
                heartbeat.abort();
                anyhow::bail!(
                    "VM creation timed out after 30 minutes. \
                     Check network connectivity and try again."
                );
            }
        }
        heartbeat.abort();
        send(&tx, "VM created with system packages", 38, InstallStage::CreateVm).await;
    } else {
        send(&tx, "VM already provisioned", 38, InstallStage::CreateVm).await;
    }

    // Ensure VM is running and reachable
    send(&tx, "Ensuring VM is running...", 39, InstallStage::BootVm).await;
    let mut vm_ok = false;
    for attempt in 1..=10 {
        match backend.exec("echo ok").await {
            Ok(o) if o.contains("ok") => { vm_ok = true; break; }
            _ => {
                if attempt == 1 || attempt == 5 {
                    backend.start().await.ok();
                }
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
    if !vm_ok {
        anyhow::bail!("VM is not reachable after 10 attempts. Check sandbox status.");
    }

    // Apply mirrors inside the running VM (more reliable than provision-time)
    if !mirrors_config.is_default() {
        send(&tx, "Configuring package mirrors...", 39, InstallStage::ConfigureProxy).await;
        mirrors::apply_mirrors(backend.as_ref(), &mirrors_config).await?;
    }

    // ---- Step 3: Install claw via background script + polling ----
    let claw_installed = backend.exec(&format!("which {} 2>/dev/null", desc.cli_binary)).await
        .map(|o| !o.trim().is_empty()).unwrap_or(false);

    if !claw_installed {
        send(&tx, &format!("Installing {} (5-10 min, runs in background)...", desc.display_name), 40, InstallStage::InstallOpenClaw).await;
        vm_background_install(backend.as_ref(), &tx, &desc.npm_install_verbose_cmd(&opts.claw_version), &desc.display_name).await?;
        send(&tx, &format!("{} installed", desc.display_name), 70, InstallStage::InstallOpenClaw).await;
    } else {
        send(&tx, &format!("{} already installed", desc.display_name), 70, InstallStage::InstallOpenClaw).await;
    }

    let claw_version = backend.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await.unwrap_or_default();

    // ---- Step 4: Post-install config (all short exec calls) ----
    if let Some(ref api_key) = opts.api_key {
        send(&tx, "Storing API key...", 72, InstallStage::StoreApiKey).await;
        keychain::store_api_key(&opts.instance_name, api_key)?;
        if let Some(cmd) = desc.set_apikey_cmd(&shell_escape(api_key)) {
            backend.exec(&format!("{cmd} 2>/dev/null || true")).await?;
        }
    }

    // Host IP
    let host_ip = match sandbox_type {
        SandboxType::LimaAlpine | SandboxType::Wsl2Alpine => {
            let ip = network::detect_host_ip().await.unwrap_or_else(|_| "127.0.0.1".into());
            backend.exec(&format!(
                "echo 'CLAWENV_HOST_IP={ip}' | sudo tee /etc/profile.d/clawenv-host.sh > /dev/null"
            )).await?;
            ip
        }
        SandboxType::PodmanAlpine => "host.containers.internal".into(),
        SandboxType::Native => "127.0.0.1".into(),
    };

    // MCP Bridge (only if the claw supports it)
    if opts.install_mcp_bridge && desc.supports_mcp {
        send(&tx, "Installing MCP Bridge plugin...", 78, InstallStage::StartOpenClaw).await;
        let mcp_js = include_str!("../../../assets/mcp/mcp-bridge.mjs");
        let mcp_dir = "/workspace/mcp-bridge";
        backend.exec(&format!("mkdir -p {mcp_dir}")).await?;
        backend.exec(&format!("cat > {mcp_dir}/index.mjs << 'MCPEOF'\n{mcp_js}\nMCPEOF")).await?;

        let bridge_url = format!("http://{host_ip}:3100");
        let token = backend.exec(
            &format!(r#"node -e "try {{ const j = JSON.parse(require('fs').readFileSync(require('path').join(process.env.HOME||'~','.{id}','{id}.json'),'utf8')); process.stdout.write((j.gateway&&j.gateway.auth&&j.gateway.auth.token)||j.token||'') }} catch {{}}"#,
                id = desc.id)
        ).await.unwrap_or_default();
        let token = token.trim();
        if !token.is_empty() {
            if let Some(mcp_cmd) = desc.mcp_register_cmd(
                "clawenv",
                &format!("{{\"command\":\"node\",\"args\":[\"{mcp_dir}/index.mjs\",\"--bridge-url\",\"{bridge_url}\"]}}")
            ) {
                let env_prefix = format!(
                    "{id_upper}_GATEWAY_URL=ws://127.0.0.1:{p} {id_upper}_GATEWAY_TOKEN={token}",
                    id_upper = desc.id.to_uppercase(),
                    p = opts.gateway_port,
                );
                backend.exec(&format!("{env_prefix} {mcp_cmd} 2>/dev/null || true")).await?;
            }
        }
    }

    // Browser (optional, post-install via background script)
    if opts.install_browser && desc.supports_browser {
        send(&tx, "Installing browser (background)...", 80, InstallStage::InstallBrowser).await;
        vm_background_run(
            backend.as_ref(), &tx,
            "sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont",
            "Installing browser",
            80, 85, InstallStage::InstallBrowser,
        ).await?;
    }

    // Start services
    send(&tx, "Starting services...", 88, InstallStage::StartOpenClaw).await;
    let ttyd_port = allocate_port(opts.gateway_port, 1);
    backend.exec(&format!(
        "nohup ttyd -p {ttyd_port} -W -i 0.0.0.0 sh -c 'cd; exec /bin/sh -l' > /tmp/ttyd.log 2>&1 &"
    )).await?;
    let gateway_cmd = desc.gateway_start_cmd(opts.gateway_port);
    backend.exec(&format!(
        "nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
    )).await?;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // ---- Post-install verification ----
    send(&tx, "Verifying installation...", 90, InstallStage::StartOpenClaw).await;
    let verify = backend.exec(&format!("which {} 2>/dev/null", desc.cli_binary)).await
        .map(|o| !o.trim().is_empty()).unwrap_or(false);
    if !verify {
        anyhow::bail!(
            "{} binary not found after installation. The install may have failed silently. \
             Check sandbox logs or try reinstalling.",
            desc.display_name
        );
    }

    // ---- Step 5: Save config ----
    send(&tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    config.config_mut().instances.retain(|i| i.name != opts.instance_name);
    config.config_mut().instances.push(InstanceConfig {
        name: opts.instance_name.clone(),
        claw_type: opts.claw_type.clone(),
        version: claw_version.trim().to_string(),
        sandbox_type,
        sandbox_id: format!("clawenv-{}", opts.instance_name),
        created_at: chrono::Utc::now().to_rfc3339(),
        last_upgraded_at: String::new(),
        gateway: GatewayConfig {
            gateway_port: opts.gateway_port,
            ttyd_port,
            bridge_port: allocate_port(opts.gateway_port, 2),
            webchat_enabled: true,
            channels: Default::default(),
        },
        resources: ResourceConfig::default(),
        browser: BrowserConfig {
            cdp_port: allocate_port(opts.gateway_port, 3),
            vnc_ws_port: allocate_port(opts.gateway_port, 4),
            ..Default::default()
        },
        cached_latest_version: String::new(),
        cached_version_check_at: String::new(),
    });
    config.save()?;

    send(&tx, "Installation complete!", 100, InstallStage::Complete).await;
    Ok(())
}

/// Run npm install for any claw as a background script in the VM.
/// `install_cmd` is the full command, e.g., "npm install -g --loglevel verbose openclaw@latest"
/// Polls a done-marker file every 5 seconds using short exec() calls.
async fn vm_background_install(
    backend: &dyn SandboxBackend,
    tx: &mpsc::Sender<InstallProgress>,
    install_cmd: &str,
    display_name: &str,
) -> Result<()> {
    let log = "/tmp/clawenv-npm.log";
    let done = "/tmp/clawenv-npm.done";

    // Clean up any previous attempt
    backend.exec(&format!("rm -f {log} {done}")).await?;

    // Write install script — runs independently in VM
    backend.exec(&format!(
        r#"cat > /tmp/clawenv-npm.sh << 'SCRIPTEOF'
#!/bin/sh
sudo {install_cmd} > {log} 2>&1
echo $? > {done}
SCRIPTEOF
chmod +x /tmp/clawenv-npm.sh"#
    )).await?;

    // Launch in background
    backend.exec("nohup sh /tmp/clawenv-npm.sh > /dev/null 2>&1 &").await?;

    // Poll for completion
    let mut last_lines = 0usize;
    let mut elapsed = 0u64;
    let mut idle = 0u64;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        elapsed += 5;

        // Check done marker
        let done_content = backend.exec(&format!("cat {done} 2>/dev/null || echo ''")).await.unwrap_or_default();
        let done_val = done_content.trim();

        // Read only NEW lines (tail from last position)
        let new_output = backend.exec(&format!(
            "tail -n +{} {log} 2>/dev/null | head -50 || echo ''",
            last_lines + 1
        )).await.unwrap_or_default();

        let new_lines: Vec<&str> = new_output.lines()
            .filter(|l| !l.trim().is_empty())
            .collect();

        if !new_lines.is_empty() {
            idle = 0;
            last_lines += new_lines.len();
            // Show last meaningful line
            let display_line = new_lines.last().unwrap_or(&"");
            let short = if display_line.len() > 85 { &display_line[..85] } else { display_line };
            let pct = std::cmp::min(40 + (elapsed / 12) as u8, 68);
            send(tx, &format!("[{elapsed}s] {short}"), pct, InstallStage::InstallOpenClaw).await;
        } else {
            idle += 5;
            let pct = std::cmp::min(40 + (elapsed / 12) as u8, 68);
            send(tx, &format!("Installing {display_name}... ({elapsed}s)"), pct, InstallStage::InstallOpenClaw).await;
        }

        // Check completion
        if !done_val.is_empty() {
            let exit_code: i32 = done_val.parse().unwrap_or(-1);
            if let Err(e) = backend.exec("rm -f /tmp/clawenv-npm.sh").await {
                tracing::debug!("Cleanup rm npm script: {e}");
            }
            if exit_code != 0 {
                let tail = backend.exec(&format!("tail -10 {log} 2>/dev/null || echo 'no log'")).await.unwrap_or_default();
                anyhow::bail!("npm install failed (exit {exit_code}):\n{tail}");
            }
            // Clean up
            backend.exec(&format!("rm -f {log} {done}")).await.ok();
            return Ok(());
        }

        // Stall: 10 min without output
        if idle >= 600 {
            let tail = backend.exec(&format!("tail -10 {log} 2>/dev/null || echo 'no log'")).await.unwrap_or_default();
            anyhow::bail!("npm install stalled (no output for 10 min):\n{tail}");
        }
    }
}

/// Run any command as background script in VM with polling and idle detection.
///
/// Polls log file for new lines every 5s. If no new output appears for 10 minutes,
/// the operation is considered stalled and returns an error.
async fn vm_background_run(
    backend: &dyn SandboxBackend,
    tx: &mpsc::Sender<InstallProgress>,
    cmd: &str,
    label: &str,
    pct_start: u8,
    pct_end: u8,
    stage: InstallStage,
) -> Result<()> {
    let log = "/tmp/clawenv-bg.log";
    let done = "/tmp/clawenv-bg.done";
    backend.exec(&format!("rm -f {log} {done}")).await?;
    backend.exec(&format!(
        "nohup sh -c '{cmd} > {log} 2>&1; echo $? > {done}' > /dev/null 2>&1 &"
    )).await?;

    let mut elapsed = 0u64;
    let mut idle = 0u64;
    let mut last_lines = 0usize;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        elapsed += 5;

        // Check done marker
        let d = backend.exec(&format!("cat {done} 2>/dev/null || echo ''")).await.unwrap_or_default();
        if !d.trim().is_empty() {
            let rc: i32 = d.trim().parse().unwrap_or(-1);
            backend.exec(&format!("rm -f {log} {done}")).await.ok();
            if rc != 0 {
                let tail = backend.exec(&format!("tail -5 {log} 2>/dev/null")).await.unwrap_or_default();
                anyhow::bail!("{label} failed (exit {rc}):\n{tail}");
            }
            return Ok(());
        }

        // Read new log lines for activity detection
        let new_output = backend.exec(&format!(
            "tail -n +{} {log} 2>/dev/null | head -20 || echo ''",
            last_lines + 1
        )).await.unwrap_or_default();
        let new_count = new_output.lines().filter(|l| !l.trim().is_empty()).count();

        if new_count > 0 {
            idle = 0;
            last_lines += new_count;
            let pct = std::cmp::min(pct_start + (elapsed / 10) as u8, pct_end);
            let last_line = new_output.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("");
            let short = if last_line.len() > 80 { &last_line[..80] } else { last_line };
            send(tx, &format!("[{elapsed}s] {short}"), pct, stage.clone()).await;
        } else {
            idle += 5;
            let pct = std::cmp::min(pct_start + (elapsed / 10) as u8, pct_end);
            send(tx, &format!("{label}... ({elapsed}s)"), pct, stage.clone()).await;
        }

        // Idle timeout: 10 minutes without new output
        if idle >= 600 {
            let tail = backend.exec(&format!("tail -10 {log} 2>/dev/null || echo 'no log'")).await.unwrap_or_default();
            anyhow::bail!("{label} stalled — no output for 10 minutes:\n{tail}");
        }
    }
}

pub async fn send(tx: &mpsc::Sender<InstallProgress>, message: &str, percent: u8, stage: InstallStage) {
    let _ = tx.send(InstallProgress {
        message: message.to_string(),
        percent,
        stage,
    }).await;
}
