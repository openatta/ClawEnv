use clawops_core::browser::{BrowserBackend, BrowserStatus, ChromiumBackend};
use clawops_core::config_loader;
use clawops_core::instance::InstanceRegistry;
use clawops_core::sandbox_backend::SandboxBackend;
use clawops_core::wire::{SandboxListResponse, SandboxVmInfo};
use std::sync::Arc;
use tauri::Emitter;

use crate::cli_bridge;
use crate::instance_helper;
use crate::ipc::emit::{emit_instance_changed, InstanceAction, InstanceChanged};
#[cfg(target_os = "windows")]
use crate::util::silent_cmd;

#[tauri::command]
pub async fn list_sandbox_vms() -> Result<Vec<SandboxVmInfo>, String> {
    let data = cli_bridge::run_cli(&["sandbox", "list"]).await.map_err(|e| e.to_string())?;
    let resp: SandboxListResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp.vms)
}

#[tauri::command]
pub async fn get_sandbox_disk_usage() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let lima_dir = clawops_core::paths::lima_home();
        let output = tokio::process::Command::new("du")
            .args(["-sh", &lima_dir.to_string_lossy()])
            .output().await.map_err(|e| e.to_string())?;
        let s = String::from_utf8_lossy(&output.stdout);
        Ok(s.split_whitespace().next().unwrap_or("unknown").to_string())
    }
    #[cfg(target_os = "windows")]
    {
        // Measure the WSL distro storage directory
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
        let wsl_dir = format!("{}/.clawenv/wsl", home);
        let path = std::path::Path::new(&wsl_dir);
        if path.exists() {
            // Use PowerShell to get directory size
            let output = silent_cmd("powershell")
                .args(["-Command", &format!(
                    "(Get-ChildItem -Recurse '{}' -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum / 1GB | ForEach-Object {{ '{{0:N1}} GB' -f $_ }}",
                    wsl_dir.replace('/', "\\")
                )])
                .output().await.map_err(|e| e.to_string())?;
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if s.is_empty() { Ok("0 GB".to_string()) } else { Ok(s) }
        } else {
            Ok("0 GB".to_string())
        }
    }
}

/// Given a VM name (e.g. `clawenv-a1b2c3d4e5f6` — an auto-generated hash, NOT
/// the user-chosen instance name), look it up against config.toml's
/// `sandbox_id` field and return the user-chosen instance `name` if found.
///
/// Used by the VM-management page to route cascade-delete through the
/// instance-level cleanup. `vm.name == instance.sandbox_id` is the correct
/// mapping — the previous implementation incorrectly assumed
/// `vm.name == "clawenv-" + instance.name`, which silently failed for every
/// install because `sandbox_id` contains a hash.
/// Pure mapping: given a VM name and the known instance list, return the
/// user-chosen instance.name whose sandbox_id matches. Extracted from the
/// IPC wrapper so the mapping rule can be unit-tested without touching disk.
fn match_instance_by_sandbox_id<I>(vm_name: &str, instances: I) -> Option<String>
where
    I: IntoIterator<Item = (String, String)>,  // (name, sandbox_id) pairs
{
    for (name, sandbox_id) in instances {
        if sandbox_id == vm_name {
            return Some(name);
        }
    }
    None
}

async fn managed_instance_for_vm(vm_name: &str) -> Option<String> {
    let registry = InstanceRegistry::with_default_path();
    let instances = match registry.list().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("managed_instance_for_vm: failed to load registry: {e}");
            return None;
        }
    };
    let pairs = instances.iter()
        .map(|i| (i.name.clone(), i.sandbox_instance.clone()));
    let hit = match_instance_by_sandbox_id(vm_name, pairs);
    // A managed-looking VM (clawenv-/ClawEnv- prefix) with no matching
    // sandbox_instance in the registry points at a genuine data inconsistency:
    // either the registry got corrupted, or an install aborted before
    // save_config. Log it so the cascade-delete no-op is visible.
    if hit.is_none()
        && (vm_name.starts_with("clawenv-") || vm_name.starts_with("ClawEnv-"))
    {
        tracing::warn!(
            "managed_instance_for_vm: VM '{}' looks managed but has no matching \
             instance.sandbox_instance in registry — falling back to raw VM action \
             (cascade delete will not update registry)",
            vm_name
        );
    }
    hit
}

#[cfg(test)]
mod match_instance_tests {
    use super::match_instance_by_sandbox_id;

    fn inst(name: &str, sandbox_id: &str) -> (String, String) {
        (name.to_string(), sandbox_id.to_string())
    }

    #[test]
    fn matches_by_sandbox_id_not_name() {
        // The VM is named "clawenv-a1b2c3" (the sandbox_id), and the instance
        // it corresponds to is called "default" — not "a1b2c3".
        let instances = vec![inst("default", "clawenv-a1b2c3")];
        assert_eq!(
            match_instance_by_sandbox_id("clawenv-a1b2c3", instances),
            Some("default".to_string())
        );
    }

    #[test]
    fn strip_prefix_lookup_would_have_failed() {
        // Regression guard for the old (broken) "strip clawenv- prefix then
        // match by name" behaviour: if someone reintroduces that logic,
        // this pair would give them "a1b2c3" which doesn't exist as a name.
        let instances = vec![inst("default", "clawenv-a1b2c3")];
        assert_eq!(
            match_instance_by_sandbox_id("clawenv-other-id", instances.clone()),
            None,
            "a VM name that looks managed but doesn't match any sandbox_id must not match"
        );
        assert_ne!(
            match_instance_by_sandbox_id("clawenv-a1b2c3", instances),
            Some("a1b2c3".into()),
            "must not return the stripped VM name as if it were the instance name"
        );
    }

    #[test]
    fn returns_none_for_orphan_vm() {
        let instances = vec![inst("default", "clawenv-zzz")];
        assert_eq!(
            match_instance_by_sandbox_id("clawenv-orphan", instances),
            None
        );
    }

    #[test]
    fn picks_first_matching_sandbox_id_when_names_differ() {
        // sandbox_id should be unique in practice, but the helper should pick
        // the first match deterministically regardless.
        let instances = vec![
            inst("first",  "clawenv-dup"),
            inst("second", "clawenv-dup"),
        ];
        assert_eq!(
            match_instance_by_sandbox_id("clawenv-dup", instances),
            Some("first".into())
        );
    }
}

/// Perform an action on a sandbox VM (start/stop/delete).
///
/// For `delete` on a managed VM (one whose name corresponds to a tracked
/// instance), this delegates to `delete_instance_with_progress` so config.toml
/// is kept consistent — deleting only the VM would leave an orphan instance
/// entry that the Home/ClawPage keep showing as "stopped forever".
#[tauri::command]
pub async fn sandbox_vm_action(app: tauri::AppHandle, vm_name: String, action: String) -> Result<(), String> {
    if action == "delete" {
        if let Some(instance_name) = managed_instance_for_vm(&vm_name).await {
            // Route through the cascade-delete path: stops the VM, deletes files,
            // removes the instance from registry, and emits the full event chain.
            return super::instance::delete_instance_with_progress(app, instance_name).await;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let args = match action.as_str() {
            "start" => vec!["start", &vm_name],
            "stop" => vec!["stop", &vm_name],
            "delete" => vec!["delete", "--force", &vm_name],
            _ => return Err(format!("Unknown action: {action}")),
        };
        let status = tokio::process::Command::new(clawops_core::paths::limactl_bin())
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("limactl {} {} failed", action, vm_name));
        }
    }
    #[cfg(target_os = "windows")]
    {
        match action.as_str() {
            "start" => {
                silent_cmd("wsl")
                    .args(["--distribution", &vm_name])
                    .stdout(std::process::Stdio::null())
                    .status().await.map_err(|e| e.to_string())?;
            }
            "stop" => {
                silent_cmd("wsl")
                    .args(["--terminate", &vm_name])
                    .status().await.map_err(|e| e.to_string())?;
            }
            "delete" => {
                silent_cmd("wsl")
                    .args(["--unregister", &vm_name])
                    .status().await.map_err(|e| e.to_string())?;
            }
            _ => return Err(format!("Unknown action: {action}")),
        }
    }
    tracing::info!("Sandbox VM action: {} on {}", action, vm_name);

    // Emit instance-changed for managed VMs so the frontend refreshes health
    // and button state. Orphan VMs have no registry entry and are a no-op here.
    if let Some(inst_name) = managed_instance_for_vm(&vm_name).await {
        let act = match action.as_str() {
            "start" => Some(InstanceAction::Start),
            "stop" => Some(InstanceAction::Stop),
            _ => None,
        };
        if let Some(a) = act {
            emit_instance_changed(&app, InstanceChanged::simple(a, inst_name));
        }
    }

    Ok(())
}

async fn resolve_backend(name: &str) -> Result<(clawops_core::instance::InstanceConfig, Arc<dyn SandboxBackend>), String> {
    let registry = InstanceRegistry::with_default_path();
    let inst = registry.find(name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Instance '{name}' not found"))?;
    let backend_arc = instance_helper::backend_arc_for_instance(&inst)?;
    Ok((inst, backend_arc))
}

/// Check if Chromium is installed in a sandbox instance.
#[tauri::command]
pub async fn check_chromium_installed(name: String) -> Result<bool, String> {
    let (_, backend) = resolve_backend(&name).await?;
    let result = backend.exec("which chromium 2>/dev/null || which chromium-browser 2>/dev/null || echo ''").await;
    match result {
        Ok(out) => Ok(!out.trim().is_empty()),
        Err(_) => Ok(false),
    }
}

/// Install Chromium + noVNC in a running sandbox instance.
///
/// v2 doesn't yet expose a per-line streaming exec on the bare
/// `SandboxBackend` trait (only `ExecutionContext` does), so this
/// runs the apk install as a single blocking exec and emits one
/// "in progress" + one "done" event. Live log streaming will return
/// when the install path is moved onto `ExecutionContext`.
#[tauri::command]
pub async fn install_chromium(
    app: tauri::AppHandle,
    name: String,
) -> Result<(), String> {
    let (_, backend) = resolve_backend(&name).await?;

    let already = backend.exec("which chromium 2>/dev/null || which chromium-browser 2>/dev/null || echo ''").await.unwrap_or_default();
    if !already.trim().is_empty() {
        let _ = app.emit("chromium-install-progress", "Chromium is already installed!");
        return Ok(());
    }

    let _ = app.emit("chromium-install-progress", "Installing Chromium and dependencies (~630MB)...");
    let _ = app.emit("chromium-install-progress", "Note: apk will resume from any previously downloaded packages.");

    let result = backend.exec(
        "sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont 2>&1 || apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont 2>&1",
    ).await;

    match result {
        Ok(output) => {
            for line in output.lines() {
                let _ = app.emit("chromium-install-progress", line);
            }
            let _ = app.emit("chromium-install-progress", "✓ Chromium installed successfully!");
            let _ = app.emit("chromium-install-complete", &name);
            emit_instance_changed(&app, InstanceChanged::simple(InstanceAction::InstallChromium, &name));
            tracing::info!("Chromium installed in '{}': {} chars output", name, output.len());
            Ok(())
        }
        Err(e) => {
            let _ = app.emit("chromium-install-progress", &format!("✗ Installation failed: {e}"));
            let _ = app.emit("chromium-install-failed", e.to_string());
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub async fn browser_status(name: String) -> Result<serde_json::Value, String> {
    let (_, backend) = resolve_backend(&name).await?;
    let browser = ChromiumBackend::new(backend);
    let status: BrowserStatus = browser.status().await.map_err(|e| e.to_string())?;
    serde_json::to_value(&status).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_start_interactive(name: String) -> Result<String, String> {
    let (inst, backend) = resolve_backend(&name).await?;
    let vnc_port = inst.browser.vnc_ws_port;
    let browser = ChromiumBackend::new(backend);
    let url = browser.start_interactive(vnc_port).await.map_err(|e| e.to_string())?;
    Ok(url)
}

#[tauri::command]
pub async fn browser_resume_headless(name: String) -> Result<(), String> {
    let (_, backend) = resolve_backend(&name).await?;
    let browser = ChromiumBackend::new(backend);
    browser.resume_headless().await.map_err(|e| e.to_string())
}

fn bridge_port() -> Result<u16, String> {
    Ok(config_loader::load_global().map_err(|e| e.to_string())?.bridge.port)
}

#[tauri::command]
pub async fn hil_complete() -> Result<(), String> {
    let port = bridge_port()?;
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/hil/complete"))
        .send().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn exec_approve() -> Result<(), String> {
    let port = bridge_port()?;
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/exec/approve"))
        .send().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn exec_deny() -> Result<(), String> {
    let port = bridge_port()?;
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/exec/deny"))
        .send().await.map_err(|e| e.to_string())?;
    Ok(())
}
