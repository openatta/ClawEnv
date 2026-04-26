use anyhow::{anyhow, Result};

use crate::claw::ClawRegistry;
use crate::config::{ConfigManager, InstanceConfig};
use crate::monitor::{InstanceHealth, InstanceMonitor};
use crate::platform::{network, process};
use crate::sandbox::{
    native_backend, LimaBackend, PodmanBackend, WslBackend, SandboxBackend, SandboxType,
};

/// Kill native gateway process — handles both the process kill AND
/// any user-scope launchd agent the claw installed to self-respawn.
/// Wrapper kept for existing internal callers; new code should pass
/// `claw_id` directly via `kill_native_gateway_public`.
async fn kill_native_gateway(claw_id: &str, port: u16) {
    kill_native_gateway_public(claw_id, port).await;
}

/// Stop a native claw's gateway process and any self-registered
/// auto-respawn agent.
///
/// macOS in particular: OpenClaw (and claws that follow its pattern)
/// registers itself with launchd as e.g. `ai.openclaw.gateway`. A
/// plain `pkill` just kicks launchd into spawning a fresh gateway
/// instantly — from the user's point of view the Stop button does
/// nothing. Always bootout the launchd agent FIRST, then pkill to
/// catch any straggler that was mid-spawn.
///
/// Windows: no launchd equivalent; just `taskkill /f /im node.exe`
/// (native mode pins a single instance, so this is safe). Linux:
/// pkill by command-line pattern — systemd --user auto-respawn is
/// possible but rare for claws we ship; if it becomes an issue we'll
/// add `systemctl --user stop` here the same way we added launchctl
/// for macOS.
pub async fn kill_native_gateway_public(claw_id: &str, _port: u16) {
    #[cfg(target_os = "windows")]
    {
        let _ = claw_id; // unused on windows — node.exe catch-all
        // taskkill /f /im node.exe kills all node processes — acceptable
        // for native mode (only one native instance allowed, all node.exe
        // are ours).
        let _ = crate::platform::process::silent_cmd("taskkill")
            .args(["/f", "/im", "node.exe"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await;
    }
    #[cfg(target_os = "macos")]
    {
        // Step 1: unload any user-scope launchd agent whose label
        // mentions this claw's id. `launchctl list` output columns:
        //   PID  ExitStatus  Label
        // Typical match: "ai.openclaw.gateway" for claw_id="openclaw".
        // bootout requires the full service path: gui/<uid>/<Label>.
        // We get <uid> via libc::getuid (stdlib doesn't expose it on
        // macOS without extras). Best-effort throughout — if launchctl
        // isn't available or the agent doesn't exist, we fall through
        // to pkill.
        if let Some(uid) = current_uid().await {
            if let Ok(out) = tokio::process::Command::new("launchctl")
                .arg("list").output().await
            {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let id_lc = claw_id.to_lowercase();
                for line in stdout.lines() {
                    let label = match line.split_whitespace().nth(2) {
                        Some(l) => l,
                        None => continue,
                    };
                    if !label.to_lowercase().contains(&id_lc) {
                        continue;
                    }
                    tracing::info!(
                        target: "clawenv::launchd",
                        "bootout {label} (gui/{uid})"
                    );
                    let _ = tokio::process::Command::new("launchctl")
                        .args(["bootout", &format!("gui/{uid}/{label}")])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status().await;
                }
            }
        }
        // Step 2: pkill any straggler. Pattern is derived from claw_id
        // so different claws (hermes etc.) match too.
        let pattern = format!("{claw_id}.*gateway");
        let _ = tokio::process::Command::new("pkill")
            .args(["-9", "-f", &pattern])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await;
    }
    #[cfg(target_os = "linux")]
    {
        let pattern = format!("{claw_id}.*gateway");
        let _ = tokio::process::Command::new("pkill")
            .args(["-9", "-f", &pattern])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await;
    }
}

/// Fetch the current process's numeric UID by shelling out to `id -u`.
/// launchctl's `gui/<uid>/<label>` service path needs it. A
/// libc::getuid() call would be faster but would require adding the
/// `libc` crate as a dependency for a single number — shell-out is
/// cheap enough here (runs once per stop) and avoids the dep churn.
#[cfg(target_os = "macos")]
async fn current_uid() -> Option<u32> {
    let out = tokio::process::Command::new("id")
        .arg("-u")
        .output()
        .await
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim().parse::<u32>().ok()
}

/// Get the appropriate sandbox backend for an instance
pub fn backend_for_instance(instance: &InstanceConfig) -> Result<Box<dyn SandboxBackend>> {
    // Use sandbox_id as the actual VM/container/directory name (ASCII-safe)
    let id = &instance.sandbox_id;
    match instance.sandbox_type {
        SandboxType::LimaAlpine => Ok(Box::new(LimaBackend::new_with_vm_name(id))),
        SandboxType::Wsl2Alpine => Ok(Box::new(WslBackend::new_with_distro_name(id))),
        SandboxType::PodmanAlpine => Ok(Box::new(PodmanBackend::with_port(id, instance.gateway.gateway_port))),
        SandboxType::Native => {
            // Native: single instance, fixed directory ~/.clawenv/native/
            Ok(Box::new(native_backend("native")))
        }
    }
}

/// Start an OpenClaw instance
pub async fn start_instance(instance: &InstanceConfig) -> Result<()> {
    // PATH management is handled by ManagedShell inside NativeBackend
    let backend = backend_for_instance(instance)?;

    // Only call backend.start() if VM/container is not already running
    let already_running = backend.exec("echo ok").await.map(|o| o.contains("ok")).unwrap_or(false);
    if !already_running {
        backend.start().await?;
        for _ in 0..5 {
            if backend.exec("echo ok").await.map(|o| o.contains("ok")).unwrap_or(false) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    // Sync host IP (sandbox only)
    if instance.sandbox_type != SandboxType::Native {
        match network::sync_host_ip(backend.as_ref()).await {
            Ok(true) => tracing::info!("Host IP updated in sandbox '{}'", instance.name),
            Ok(false) => {}
            Err(e) => tracing::warn!("Failed to sync host IP: {e}"),
        }

        // Re-apply proxy via the unified resolver. Priority: per-VM
        // InstanceConfig.proxy → global config → OS detection. See
        // docs/23-proxy-architecture.md §9 (Start lifecycle).
        if let Ok(cfg) = ConfigManager::load() {
            let scope = crate::config::proxy_resolver::Scope::RuntimeSandbox {
                instance,
                backend: backend.as_ref(),
            };
            if let Some(triple) = scope.resolve(&cfg).await {
                if let Err(e) = crate::config::proxy_resolver::apply_to_sandbox(&triple, backend.as_ref()).await {
                    tracing::warn!(target: "clawenv::proxy", "apply_to_sandbox failed: {e}");
                }
            } else {
                crate::config::proxy_resolver::clear_sandbox(backend.as_ref()).await.ok();
            }
        }
    }

    let registry = ClawRegistry::load();
    let desc = registry.get(&instance.claw_type);
    let port = instance.gateway.gateway_port;

    // Start ttyd (sandbox only). Always pkill first: after a VM restart the old
    // ttyd pid is dead but the port-listen probe can race with Lima's port
    // forwarder coming up, making us skip the launch and leaving the terminal
    // button broken. Unconditional pkill → launch is cheap and eliminates the
    // race — spurious "no such process" from pkill is silenced with `|| true`.
    if instance.sandbox_type != SandboxType::Native {
        let ttyd_port = instance.gateway.ttyd_port;
        backend.exec("pkill -9 ttyd 2>/dev/null || true").await.ok();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        backend.exec(&format!(
            "nohup ttyd -p {ttyd_port} -W -i 0.0.0.0 sh -c 'cd; exec /bin/sh -l' > /tmp/ttyd.log 2>&1 &"
        )).await?;
        // Tiny settle so port-forward becomes active before the UI pokes it.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        tracing::info!("ttyd (re)started on 0.0.0.0:{ttyd_port}");
    }

    // Always kill stale gateway before starting — ensures clean state
    if instance.sandbox_type == SandboxType::Native {
        kill_native_gateway(&desc.id, port).await;
    } else {
        for pn in &desc.process_names() {
            backend.exec(&process::kill_by_name_cmd(pn)).await.ok();
        }
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // NOTE: an earlier v0.3.0 iteration purged `~/.{id}/{id}.json` here
    // to "force the gateway to regenerate a fresh token". That turned
    // out to be wrong — OpenClaw's config file carries gateway.mode +
    // other bootstrap state, not just the token, and deleting it makes
    // the gateway refuse to start ("Gateway start blocked: existing
    // config is missing gateway.mode", even with --allow-unconfigured).
    // The real token-mismatch fix lives in the install flow's `init_cmd`
    // invocation (see ClawDescriptor.init_cmd / install.rs), which
    // properly seeds gateway.mode via `openclaw config set` instead of
    // nuking the file.

    // For Lima: bind gateway on 0.0.0.0 so guestagent can detect and forward the port.
    // Use --allow-unconfigured flag instead of config set to avoid ConfigMutationConflictError.
    // Port is passed via --port flag in gateway_start_cmd, no need to modify openclaw config.
    if instance.sandbox_type == SandboxType::LimaAlpine {
        backend.exec(&format!(
            "{bin} config set gateway.bind lan 2>/dev/null; {bin} config set gateway.controlUi.dangerouslyAllowHostHeaderOriginFallback true 2>/dev/null || true",
            bin = desc.cli_binary
        )).await.ok();
        // Brief pause to let config flush
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    if let Some(gateway_cmd) = desc.gateway_start_cmd(port) {
        if instance.sandbox_type == SandboxType::Native {
            // Native: use ManagedShell::spawn_detached for a truly independent process
            // with correct PATH (our own node/git, not system)
            let shell = crate::platform::managed_shell::ManagedShell::new();
            let log_path = crate::config::clawenv_root()
                .join("native").join("gateway.log");

            // Parse gateway_cmd into binary + args: "openclaw gateway --port 3000 --allow-unconfigured"
            let parts: Vec<&str> = gateway_cmd.split_whitespace().collect();
            let (bin, args) = if parts.len() > 1 { (parts[0], &parts[1..]) } else { (parts[0], &[][..]) };

            shell.spawn_detached(bin, args, &log_path).await?;
        } else {
            // Sandbox: nohup inside VM/container
            backend.exec(&format!(
                "nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
            )).await?;
        }
    }

    // Dashboard process — independent of gateway. Lives on
    // instance.gateway.dashboard_port (allocated at install time). For
    // Hermes this is THE thing the user wants running; for OpenClaw
    // dashboard_port is 0 and this block is a no-op.
    let dashboard_port = instance.gateway.dashboard_port;
    if dashboard_port != 0 {
        if let Some(dashboard_cmd) = desc.dashboard_start_cmd(dashboard_port) {
            if instance.sandbox_type == SandboxType::Native {
                let shell = crate::platform::managed_shell::ManagedShell::new();
                let log_path = crate::config::clawenv_root()
                    .join("native").join("dashboard.log");
                let parts: Vec<&str> = dashboard_cmd.split_whitespace().collect();
                let (bin, args) = if parts.len() > 1 { (parts[0], &parts[1..]) } else { (parts[0], &[][..]) };
                shell.spawn_detached(bin, args, &log_path).await?;
            } else {
                backend.exec(&format!(
                    "nohup {dashboard_cmd} > /tmp/clawenv-dashboard.log 2>&1 &"
                )).await?;
            }
            tracing::info!("Dashboard started on port {dashboard_port} for '{}'", instance.name);
        }
    }

    // Wait for the user-facing process to become responsive.
    //
    // The probe target is the dashboard_port when one exists (Hermes), else
    // the gateway_port (OpenClaw). That's the port the UI's "Open Control
    // Panel" button opens, and the dot next to the instance name tracks
    // the same thing — they should all agree.
    //
    // Native Windows takes ~18-22s end-to-end: node startup + openclaw
    // ready (5 plugins) + channels/sidecars + HTTP listener bind. The old
    // 13s ceiling (3+2+2+2+2+2) timed out before the gateway port opened,
    // making the check report "gateway failed" even though it was still
    // coming up. New budget: 3 + 11*2 = 25s worst case before we declare
    // failure and snapshot the log. Hermes dashboard first-boot can also
    // take up to ~30s when it triggers an in-process Web UI rebuild (our
    // install-time pre-build avoids this, but defense-in-depth).
    let probe_port = if instance.gateway.dashboard_port != 0 {
        instance.gateway.dashboard_port
    } else {
        port
    };
    for i in 0..12 {
        tokio::time::sleep(std::time::Duration::from_secs(if i == 0 { 3 } else { 2 })).await;

        if instance.sandbox_type == SandboxType::Native {
            // Native: pure Rust HTTP probe (no shell)
            let health = InstanceMonitor::check_health_native(probe_port).await;
            if health == InstanceHealth::Running {
                tracing::info!("Instance '{}' ready on port {probe_port}", instance.name);
                return Ok(());
            }
        } else {
            // Sandbox: curl inside VM
            let check_cmd = format!(
                "curl -s -o /dev/null -w '%{{http_code}}' --connect-timeout 2 http://127.0.0.1:{probe_port}/ 2>/dev/null || echo '000'"
            );
            let check = backend.exec(&check_cmd).await.unwrap_or_default();
            let code = check.trim().trim_matches('\'');
            if code != "000" && !code.is_empty() {
                tracing::info!("Instance '{}' ready on port {probe_port} (HTTP {code})", instance.name);
                return Ok(());
            }
        }
    }

    // Gateway did not respond after 6 probes (~13s). Check if process even started.
    // Check any of the process name variants
    let mut proc_check = String::new();
    for pn in &desc.process_names() {
        let check = backend.exec(&process::check_process_cmd(pn)).await.unwrap_or_default();
        if check.contains("running") { proc_check = check; break; }
    }
    if proc_check.contains("running") {
        tracing::warn!("Instance '{}' process is running but not yet responding on port {probe_port}", instance.name);
        Ok(())
    } else {
        // Process is not running — read log for error details
        let log_cmd = if instance.sandbox_type == SandboxType::Native {
            let log_path = crate::config::clawenv_root()
                .join("native").join("gateway.log");
            #[cfg(target_os = "windows")]
            {
                format!("powershell -ExecutionPolicy Bypass -Command \"Get-Content '{}' -Tail 20 -ErrorAction SilentlyContinue\"",
                    log_path.display())
            }
            #[cfg(not(target_os = "windows"))]
            {
                format!("tail -20 '{}' 2>/dev/null || echo 'no log'", log_path.display())
            }
        } else {
            "tail -20 /tmp/clawenv-gateway.log 2>/dev/null || echo 'no log'".to_string()
        };
        let log = backend.exec(&log_cmd).await.unwrap_or_else(|_| "no log available".into());
        anyhow::bail!(
            "Gateway failed to start for '{}' on port {port}. \
             Process exited unexpectedly.\n\nGateway log:\n{log}",
            instance.name
        )
    }
}

/// Stop a claw instance — force kill all processes
pub async fn stop_instance(instance: &InstanceConfig) -> Result<()> {
    let registry = ClawRegistry::load();
    let desc = registry.get(&instance.claw_type);
    let backend = backend_for_instance(instance)?;
    let port = instance.gateway.gateway_port;

    // Kill gateway
    if instance.sandbox_type == SandboxType::Native {
        // Native: launchctl bootout any self-registered auto-respawn
        // agent first, THEN pkill. See kill_native_gateway_public for
        // why the bootout step is load-bearing on macOS.
        kill_native_gateway(&desc.id, port).await;
    } else {
        for pn in &desc.process_names() {
            backend.exec(&process::kill_by_name_cmd(pn)).await.ok();
        }
        backend.exec(&process::kill_by_name_cmd("ttyd")).await.ok();
    }
    backend.stop().await?;
    tracing::info!("Instance '{}' stopped", instance.name);
    Ok(())
}

/// Restart an OpenClaw instance
pub async fn restart_instance(instance: &InstanceConfig) -> Result<()> {
    stop_instance(instance).await.ok();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    start_instance(instance).await
}

/// Get the health status of an instance.
///
/// Probe target mirrors `start_instance`: dashboard_port when the claw
/// has a standalone dashboard (Hermes), gateway_port otherwise (OpenClaw).
/// That way the UI's green/red dot, the "Open Control Panel" button, and
/// the start-time readiness probe all observe the same port — if any one
/// of them fires, the others will too.
pub async fn instance_health(instance: &InstanceConfig) -> InstanceHealth {
    let probe_port = if instance.gateway.dashboard_port != 0 {
        instance.gateway.dashboard_port
    } else {
        instance.gateway.gateway_port
    };
    let is_native = instance.sandbox_type == SandboxType::Native;
    if is_native {
        // Native: pure Rust HTTP check (no shell needed)
        InstanceMonitor::check_health_native(probe_port).await
    } else {
        let backend = match backend_for_instance(instance) {
            Ok(b) => b,
            Err(_) => return InstanceHealth::Unreachable,
        };
        InstanceMonitor::check_health_with_port(backend.as_ref(), probe_port).await
    }
}

/// Get instance by name from config
pub fn get_instance<'a>(config: &'a ConfigManager, name: &str) -> Result<&'a InstanceConfig> {
    config
        .instances()
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| anyhow!("Instance '{}' not found", name))
}

/// One-shot legacy migration: instances installed under pre-v0.2.7 clawenv
/// don't have `gateway.dashboard_port` set (it serde-defaults to 0), which
/// silences the dashboard code path in `start_instance` — the Hermes UI
/// stays dead across upgrades until the user reinstalls or edits
/// config.toml by hand. Detect that, patch the config, AND patch the
/// Lima VM's yaml so the new port is forwarded from guest to host on the
/// next start. Idempotent: instances that already have `dashboard_port
/// != 0` or whose claw has no dashboard are no-ops.
///
/// Returns `true` iff at least one instance was mutated, so callers can
/// log something user-visible on first upgrade.
pub fn migrate_instance_ports(
    config: &mut ConfigManager,
    registry: &ClawRegistry,
) -> Result<bool> {
    // Collect the migrations up front so we're not borrowing
    // `config.instances()` immutably while also holding `&mut config` for
    // update_instance. Keep `sandbox_id` + `sandbox_type` so we know which
    // Lima VM directory (if any) to patch alongside config.
    #[derive(Clone)]
    struct Pending {
        name: String,
        new_port: u16,
        sandbox_type: crate::sandbox::SandboxType,
        sandbox_id: String,
    }
    let migrations: Vec<Pending> = config.instances()
        .iter()
        .filter_map(|inst| {
            let desc = registry.get(&inst.claw_type);
            if desc.has_dashboard()
                && inst.gateway.dashboard_port == 0
                && desc.dashboard_port_offset > 0
            {
                let new_port = inst.gateway.gateway_port
                    .saturating_add(desc.dashboard_port_offset);
                Some(Pending {
                    name: inst.name.clone(),
                    new_port,
                    sandbox_type: inst.sandbox_type,
                    sandbox_id: inst.sandbox_id.clone(),
                })
            } else {
                None
            }
        })
        .collect();

    let changed = !migrations.is_empty();
    for m in migrations {
        config.update_instance(&m.name, |i| {
            i.gateway.dashboard_port = m.new_port;
        })?;
        tracing::info!(
            "Migrated instance '{}': dashboard_port 0 → {} (pre-v0.2.7 config)",
            m.name, m.new_port
        );

        // Lima: patch the VM's lima.yaml to add a portForwards entry for
        // the new dashboard port. Without this, the guest can bind to the
        // port just fine but the host-side tunnel has no listener, so
        // `curl http://127.0.0.1:3005/` from the host fails. Applied on
        // LimaAlpine only — WSL doesn't use portForwards, Podman's port
        // binding is set at container create and would need a recreate
        // (that's a bigger surgery we defer).
        if m.sandbox_type == crate::sandbox::SandboxType::LimaAlpine {
            let yaml_path = crate::sandbox::lima_home()
                .join(&m.sandbox_id)
                .join("lima.yaml");
            if let Err(e) = patch_lima_yaml_dashboard_forward(&yaml_path, m.new_port) {
                tracing::warn!(
                    "migrate: failed to patch {}: {e} — user must restart VM or reinstall \
                     for dashboard port-forward to take effect",
                    yaml_path.display()
                );
            }
        }
    }
    Ok(changed)
}

/// Synchronous patch: load lima.yaml, run it through
/// `ensure_dashboard_port_forward`, write back. No-op if the file doesn't
/// exist (VM hasn't been created yet) or if the forward is already there.
/// `dashboard_port` comes from the authoritative InstanceConfig, not from
/// the yaml — see `ensure_dashboard_port_forward_yaml` docs for why.
fn patch_lima_yaml_dashboard_forward(
    yaml_path: &std::path::Path,
    dashboard_port: u16,
) -> Result<()> {
    if !yaml_path.exists() {
        return Ok(());
    }
    let current = std::fs::read_to_string(yaml_path)
        .map_err(|e| anyhow!("read {}: {e}", yaml_path.display()))?;
    let patched = crate::sandbox::ensure_dashboard_port_forward_yaml(&current, dashboard_port);
    if patched != current {
        std::fs::write(yaml_path, patched)
            .map_err(|e| anyhow!("write {}: {e}", yaml_path.display()))?;
        tracing::info!(
            "Patched {} to forward dashboard port {dashboard_port}",
            yaml_path.display()
        );
    }
    Ok(())
}

/// Remove an instance from config and destroy its sandbox
pub async fn remove_instance(config: &mut ConfigManager, name: &str) -> Result<()> {
    let instance = config
        .instances()
        .iter()
        .find(|i| i.name == name)
        .ok_or_else(|| anyhow!("Instance '{}' not found", name))?
        .clone();

    let backend = backend_for_instance(&instance)?;
    stop_instance(&instance).await.ok();

    // Wait for processes to fully exit before destroying files
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Force kill any remaining native gateway
    if instance.sandbox_type == SandboxType::Native {
        kill_native_gateway(&instance.claw_type, instance.gateway.gateway_port).await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    // Destroy backend (delete VM/files). On Windows the first try often
    // hits "file in use" while WSL or a related process is still releasing
    // locks, so retry after a 2s settle. Other platforms don't have this
    // pattern — don't want a dangling `mut` warning there.
    #[cfg(target_os = "windows")]
    {
        let mut destroy_result = backend.destroy().await;
        if destroy_result.is_err() {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            destroy_result = backend.destroy().await;
        }
        destroy_result?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        backend.destroy().await?;
    }

    config.remove_instance(name)?;

    // Clean up the per-instance proxy keychain entry (if any). Missing
    // entries are fine — `delete_instance_proxy_password` errors when the
    // key doesn't exist, which is the common case.
    let _ = crate::config::keychain::delete_instance_proxy_password(name);

    tracing::info!("Instance '{}' removed", name);
    Ok(())
}

/// Get the VM's external IP address
pub async fn get_sandbox_ip(instance: &InstanceConfig) -> Result<String> {
    if instance.sandbox_type == SandboxType::Native {
        return Ok("127.0.0.1".into());
    }
    let backend = backend_for_instance(instance)?;
    let output = backend.exec(
        "ip -4 addr show eth0 2>/dev/null | grep -oP 'inet \\K[0-9.]+' || hostname -i 2>/dev/null || echo '127.0.0.1'"
    ).await?;
    let ip = output.trim().to_string();
    if ip.is_empty() { Ok("127.0.0.1".into()) } else { Ok(ip) }
}
