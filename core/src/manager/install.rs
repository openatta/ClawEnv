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

/// Generate an ASCII-safe short ID for directory names.
/// Uses hex of first 6 bytes of SHA256(name + timestamp).
pub fn generate_dir_id(name: &str) -> String {
    use sha2::{Sha256, Digest};
    let input = format!("{}{}", name, chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(&hash[..6]) // 12 hex chars
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
        // Validate: some claw types (e.g., Hermes) only support sandbox installation
        let registry = ClawRegistry::load();
        if let Ok(desc) = registry.get_strict(&opts.claw_type) {
            if !desc.supports_native {
                anyhow::bail!(
                    "{} does not support native installation. Use sandbox mode instead.",
                    desc.display_name
                );
            }
        }
        return super::install_native::install_native(&opts, config, &tx).await;
    }

    // ---- Sandbox path below ----
    let dir_id = generate_dir_id(&opts.instance_name);
    let sandbox_id = format!("clawenv-{dir_id}");

    // Resolve the claw descriptor for this install
    let registry = ClawRegistry::load();
    let desc = registry.get_strict(&opts.claw_type)?;

    send(&tx, "Detecting platform...", 5, InstallStage::DetectPlatform).await;
    // Use dir_id for VM name (ASCII-safe, supports non-ASCII instance names)
    let backend: Box<dyn SandboxBackend> = detect_backend_for(&dir_id)?;

    send(&tx, &format!("Checking {} prerequisites...", backend.name()), 8, InstallStage::EnsurePrerequisites).await;
    backend.ensure_prerequisites().await?;
    send(&tx, &format!("{} ready", backend.name()), 10, InstallStage::EnsurePrerequisites).await;

    let sandbox_type = if opts.use_native { SandboxType::Native } else { SandboxType::from_os() };
    let mirrors_config = config.config().clawenv.mirrors.clone();

    // ---- Step 2: Create VM (provision = system packages only) ----
    // Check if VM exists AND has basic packages. A VM that exists but
    // wasn't fully provisioned (e.g., interrupted install) is treated as non-existent.
    let readiness_cmd = match desc.package_manager {
        crate::claw::descriptor::PackageManager::Pip | crate::claw::descriptor::PackageManager::GitPip => "python3 --version 2>/dev/null",
        crate::claw::descriptor::PackageManager::Npm => "node --version 2>/dev/null",
    };
    let vm_ready = match backend.exec(readiness_cmd).await {
        Ok(o) => {
            let t = o.trim();
            t.starts_with('v') || t.starts_with("Python")
        }
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

    // Ensure VM is running and reachable (may take longer on first boot)
    send(&tx, "Ensuring VM is running...", 39, InstallStage::BootVm).await;
    let mut vm_ok = false;
    for attempt in 1..=20 {
        match backend.exec("echo ok").await {
            Ok(o) if o.contains("ok") => { vm_ok = true; break; }
            _ => {
                if attempt == 1 || attempt == 5 || attempt == 10 {
                    backend.start().await.ok();
                }
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
    if !vm_ok {
        anyhow::bail!("VM is not reachable after 20 attempts. Check sandbox status.");
    }

    // Apply mirrors inside the running VM (more reliable than provision-time)
    if !mirrors_config.is_default() {
        send(&tx, "Configuring package mirrors...", 39, InstallStage::ConfigureProxy).await;
        mirrors::apply_mirrors(backend.as_ref(), &mirrors_config).await?;
    }

    // ---- Step 2b: Install extra sandbox packages required by this claw type ----
    if !desc.sandbox_provision.is_empty() {
        let pkgs = desc.sandbox_provision.join(" ");
        send(&tx, &format!("Installing {} dependencies ({pkgs})...", desc.display_name), 39, InstallStage::InstallOpenClaw).await;
        backend.exec(&format!("sudo apk add --no-cache {pkgs} 2>&1 || true")).await?;
    }

    // ---- Step 3: Install claw via background script + polling ----
    let claw_installed = backend.exec(&format!("which {} 2>/dev/null", desc.cli_binary)).await
        .map(|o| !o.trim().is_empty()).unwrap_or(false);

    if !claw_installed {
        send(&tx, &format!("Installing {} (5-10 min, runs in background)...", desc.display_name), 40, InstallStage::InstallOpenClaw).await;
        vm_background_install(backend.as_ref(), &tx, &desc.sandbox_install_cmd(&opts.claw_version), &desc.display_name).await?;
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

    // Hermes API Server: configure .env + install fastapi/uvicorn workaround
    if desc.uses_python_mcp() && !desc.gateway_cmd.is_empty() {
        send(&tx, "Configuring Hermes API Server...", 76, InstallStage::StartOpenClaw).await;
        // Enable API Server in ~/.hermes/.env (idempotent: check before appending)
        backend.exec(&format!(
            "mkdir -p ~/.{id}; grep -q 'API_SERVER_ENABLED' ~/.{id}/.env 2>/dev/null || printf 'API_SERVER_ENABLED=true\\nAPI_SERVER_KEY=clawenv-local\\n' >> ~/.{id}/.env",
            id = desc.id
        )).await?;
        // Workaround: [web] extra uv.lock bug (NousResearch/hermes-agent#9569)
        backend.exec("pip install --break-system-packages fastapi \"uvicorn[standard]\" 2>/dev/null || true").await?;
    }

    // MCP plugins (Bridge + HIL skill + HW Notify)
    if opts.install_mcp_bridge && desc.supports_mcp {
        send(&tx, "Installing plugins (MCP Bridge + HIL Skill + HW Notify)...", 78, InstallStage::StartOpenClaw).await;
        let bridge_url = format!("http://{host_ip}:{}", allocate_port(opts.gateway_port, 2));

        let use_python = desc.uses_python_mcp();
        let mcp_runner = if use_python { "python3" } else { "node" };

        // Plugin definitions: (dir_name, reg_name, file_name, content)
        let plugins: Vec<(&str, &str, &str, &str)> = if use_python {
            vec![
                ("mcp-bridge", "clawenv",     "bridge.py", include_str!("../../../assets/mcp/mcp-bridge.py")),
                ("hil-skill",  "clawenv-hil", "skill.py",  include_str!("../../../assets/mcp/hil-skill.py")),
                ("hw-notify",  "hw-notify",   "notify.py", include_str!("../../../assets/mcp/hw-notify.py")),
            ]
        } else {
            vec![
                ("mcp-bridge", "clawenv",     "index.mjs", include_str!("../../../assets/mcp/mcp-bridge.mjs")),
                ("hil-skill",  "clawenv-hil", "index.mjs", include_str!("../../../assets/mcp/hil-skill.mjs")),
                ("hw-notify",  "hw-notify",   "index.mjs", include_str!("../../../assets/mcp/hw-notify.mjs")),
            ]
        };

        // Deploy all plugin scripts into sandbox
        for (dir_name, _, file_name, content) in &plugins {
            let dir = format!("/workspace/{dir_name}");
            backend.exec(&format!("mkdir -p {dir}")).await?;
            let eof_marker = format!("EOF_{}", dir_name.to_uppercase().replace('-', "_"));
            backend.exec(&format!("cat > {dir}/{file_name} << '{eof_marker}'\n{content}\n{eof_marker}")).await?;
        }

        // Python runtime: install MCP SDK
        if use_python {
            let pip_result = backend.exec("pip install --break-system-packages mcp httpx 2>&1").await;
            match &pip_result {
                Ok(output) if output.contains("ERROR") || output.contains("error:") => {
                    tracing::warn!("MCP SDK pip install may have failed: {}", output.lines().last().unwrap_or(""));
                }
                Err(e) => {
                    tracing::warn!("MCP SDK pip install failed: {e} — MCP Bridge may not work");
                }
                _ => {}
            }
        }

        // Read gateway token for registration
        let token = if !use_python {
            let t = backend.exec(
                &format!(r#"node -e "try {{ const j = JSON.parse(require('fs').readFileSync(require('path').join(process.env.HOME||'~','.{id}','{id}.json'),'utf8')); process.stdout.write((j.gateway&&j.gateway.auth&&j.gateway.auth.token)||j.token||'') }} catch {{}}"#,
                    id = desc.id)
            ).await.unwrap_or_default();
            t.trim().to_string()
        } else {
            let t = backend.exec(
                &format!(r#"python3 -c "
import json, os, pathlib
p = pathlib.Path.home() / '.{id}' / 'config.json'
if p.exists():
    d = json.loads(p.read_text())
    print(d.get('token', d.get('gateway', {{}}).get('auth', {{}}).get('token', '')), end='')
" 2>/dev/null"#, id = desc.id)
            ).await.unwrap_or_default();
            t.trim().to_string()
        };

        // Register all plugins in one loop
        let env_prefix = if !token.is_empty() {
            format!(
                "{id_upper}_GATEWAY_URL=ws://127.0.0.1:{p} {id_upper}_GATEWAY_TOKEN={token} ",
                id_upper = desc.id.to_uppercase(),
                p = opts.gateway_port,
            )
        } else {
            String::new()
        };

        for (dir_name, reg_name, file_name, _) in &plugins {
            let entry = format!("/workspace/{dir_name}/{file_name}");
            if let Some(cmd) = desc.mcp_register_cmd(
                reg_name,
                &format!("{{\"command\":\"{mcp_runner}\",\"args\":[\"{entry}\",\"--bridge-url\",\"{bridge_url}\"]}}")
            ) {
                backend.exec(&format!("{env_prefix}{cmd} 2>/dev/null || true")).await?;
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
    if let Some(gateway_cmd) = desc.gateway_start_cmd(opts.gateway_port) {
        backend.exec(&format!(
            "nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
        )).await?;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

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

    // ---- Step 5: Save config (load-merge-write to avoid race with concurrent installs) ----
    send(&tx, "Saving configuration...", 92, InstallStage::SaveConfig).await;
    config.save_instance(InstanceConfig {
        name: opts.instance_name.clone(),
        claw_type: opts.claw_type.clone(),
        version: claw_version.trim().to_string(),
        sandbox_type,
        sandbox_id: sandbox_id.clone(),
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
    })?;

    send(&tx, "Installation complete!", 100, InstallStage::Complete).await;
    Ok(())
}

/// Run package install as a background script in the VM.
/// Delegates to the shared `background::run_background_script` module.
async fn vm_background_install(
    backend: &dyn SandboxBackend,
    tx: &mpsc::Sender<InstallProgress>,
    install_cmd: &str,
    display_name: &str,
) -> Result<()> {
    use super::background::{run_background_script, BackgroundScriptOpts};
    let tx = tx.clone();
    run_background_script(backend, &BackgroundScriptOpts {
        cmd: install_cmd,
        label: &format!("Installing {display_name}"),
        sudo: true,
        log_file: "/tmp/clawenv-install.log",
        done_file: "/tmp/clawenv-install.done",
        script_file: "/tmp/clawenv-install.sh",
        pct_range: (40, 68),
        ..Default::default()
    }, move |msg, pct| {
        let tx = tx.clone();
        tokio::spawn(async move {
            send(&tx, &msg, pct, InstallStage::InstallOpenClaw).await;
        });
    }).await
}

/// Run any command as a background script in the VM with progress polling.
/// Delegates to the shared `background::run_background_script` module.
async fn vm_background_run(
    backend: &dyn SandboxBackend,
    tx: &mpsc::Sender<InstallProgress>,
    cmd: &str,
    label: &str,
    pct_start: u8,
    pct_end: u8,
    stage: InstallStage,
) -> Result<()> {
    use super::background::{run_background_script, BackgroundScriptOpts};
    let tx = tx.clone();
    run_background_script(backend, &BackgroundScriptOpts {
        cmd,
        label,
        sudo: false,
        log_file: "/tmp/clawenv-bg.log",
        done_file: "/tmp/clawenv-bg.done",
        script_file: "/tmp/clawenv-bg.sh",
        pct_range: (pct_start, pct_end),
        ..Default::default()
    }, move |msg, pct| {
        let tx = tx.clone();
        let stage = stage.clone();
        tokio::spawn(async move {
            send(&tx, &msg, pct, stage).await;
        });
    }).await
}

pub async fn send(tx: &mpsc::Sender<InstallProgress>, message: &str, percent: u8, stage: InstallStage) {
    let _ = tx.send(InstallProgress {
        message: message.to_string(),
        percent,
        stage,
    }).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_escape_basic() {
        assert_eq!(shell_escape("hello"), "hello");
        assert_eq!(shell_escape("it's"), "it'\\''s");
        assert_eq!(shell_escape("a'b'c"), "a'\\''b'\\''c");
    }

    #[test]
    fn test_shell_escape_empty() {
        assert_eq!(shell_escape(""), "");
    }

    #[test]
    fn test_validate_instance_name_valid() {
        assert!(validate_instance_name("default").is_ok());
        assert!(validate_instance_name("my-instance").is_ok());
        assert!(validate_instance_name("test_123").is_ok());
        assert!(validate_instance_name("a").is_ok());
    }

    #[test]
    fn test_validate_instance_name_invalid() {
        assert!(validate_instance_name("").is_err());
        assert!(validate_instance_name("has space").is_err());
        assert!(validate_instance_name("has.dot").is_err());
        assert!(validate_instance_name(&"x".repeat(64)).is_err());
    }

    #[test]
    fn test_generate_dir_id_format() {
        let id = generate_dir_id("test");
        assert_eq!(id.len(), 12, "dir_id should be 12 hex chars");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "should be hex: {id}");
    }

    #[test]
    fn test_generate_dir_id_unique() {
        let id1 = generate_dir_id("test");
        std::thread::sleep(std::time::Duration::from_millis(1));
        let id2 = generate_dir_id("test");
        assert_ne!(id1, id2, "different timestamps should produce different IDs");
    }

    #[test]
    fn test_allocate_port_basic() {
        // allocate_port tries base+offset, returns it if free
        let port = allocate_port(3000, 1);
        assert!(port >= 3001 && port <= 3019, "should be in block: {port}");
    }

    #[test]
    fn test_allocate_port_range() {
        // allocate_port(base, offset) should return within the 20-port block
        let port = allocate_port(50000, 2);
        assert!(port >= 50002 && port <= 50019, "should be in block: {port}");
    }

    #[test]
    fn test_install_options_defaults() {
        let opts = InstallOptions::default();
        assert_eq!(opts.instance_name, "default");
        assert_eq!(opts.claw_type, "openclaw");
        assert_eq!(opts.claw_version, "latest");
        assert!(!opts.install_browser);
        assert!(opts.install_mcp_bridge);
        assert!(!opts.use_native);
    }
}
