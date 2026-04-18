use clawenv_core::api::SandboxListResponse;
use clawenv_core::browser::chromium::ChromiumBackend;
use clawenv_core::browser::BrowserBackend;
use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
#[cfg(target_os = "windows")]
use clawenv_core::platform::process::silent_cmd;
use std::sync::Arc;
use tauri::Emitter;

use crate::cli_bridge;
use crate::ipc::emit::{emit_instance_changed, InstanceAction, InstanceChanged};

#[tauri::command]
pub async fn list_sandbox_vms() -> Result<Vec<clawenv_core::api::SandboxVmInfo>, String> {
    let data = cli_bridge::run_cli(&["sandbox", "list"]).await.map_err(|e| e.to_string())?;
    let resp: SandboxListResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp.vms)
}

#[tauri::command]
pub async fn get_sandbox_disk_usage() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let lima_dir = clawenv_core::sandbox::lima_home();
        let output = tokio::process::Command::new("du")
            .args(["-sh", &lima_dir.to_string_lossy()])
            .output().await.map_err(|e| e.to_string())?;
        let s = String::from_utf8_lossy(&output.stdout);
        Ok(s.split_whitespace().next().unwrap_or("unknown").to_string())
    }
    #[cfg(target_os = "linux")]
    {
        // Report the size of the private Podman data dir directly — matches
        // Lima (macOS) and WSL (Windows), which also `du`/PowerShell the
        // ClawEnv-owned directory rather than asking the tool about global
        // state. `podman system df` would count any containers the user
        // created outside of ClawEnv, inflating the number.
        let data_dir = clawenv_core::sandbox::podman_data_home();
        let output = tokio::process::Command::new("du")
            .args(["-sh", &data_dir.to_string_lossy()])
            .output().await.map_err(|e| e.to_string())?;
        let s = String::from_utf8_lossy(&output.stdout);
        Ok(s.split_whitespace().next().unwrap_or("0").to_string())
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

fn managed_instance_for_vm(vm_name: &str) -> Option<String> {
    let config = match ConfigManager::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("managed_instance_for_vm: failed to load config: {e}");
            return None;
        }
    };
    let pairs = config.instances().iter()
        .map(|i| (i.name.clone(), i.sandbox_id.clone()));
    let hit = match_instance_by_sandbox_id(vm_name, pairs);
    // A managed-looking VM (clawenv-/ClawEnv- prefix) with no matching
    // sandbox_id in config.toml points at a genuine data inconsistency:
    // either the config got corrupted, or an install aborted before
    // save_config. Log it so the cascade-delete no-op is visible in the
    // tracing backend instead of silently falling through to raw VM delete.
    if hit.is_none()
        && (vm_name.starts_with("clawenv-") || vm_name.starts_with("ClawEnv-"))
    {
        tracing::warn!(
            "managed_instance_for_vm: VM '{}' looks managed but has no matching \
             instance.sandbox_id in config.toml — falling back to raw VM action \
             (cascade delete will not update config)",
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
        if let Some(instance_name) = managed_instance_for_vm(&vm_name) {
            // Route through the cascade-delete path: stops the VM, deletes files,
            // removes the instance from config.toml, and emits the full event chain.
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
        let status = tokio::process::Command::new(clawenv_core::sandbox::limactl_bin())
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("limactl {} {} failed", action, vm_name));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let args = match action.as_str() {
            "start" => vec!["start".to_string(), vm_name.clone()],
            "stop" => vec!["stop".to_string(), vm_name.clone()],
            "delete" => vec!["rm".to_string(), "-f".to_string(), vm_name.clone()],
            _ => return Err(format!("Unknown action: {action}")),
        };
        let status = tokio::process::Command::new("podman")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("podman {} {} failed", action, vm_name));
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
    // and button state. Orphan VMs have no config entry and are a no-op here.
    if let Some(inst_name) = managed_instance_for_vm(&vm_name) {
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

/// Check if Chromium is installed in a sandbox instance
#[tauri::command]
pub async fn check_chromium_installed(name: String) -> Result<bool, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let result = backend.exec("which chromium 2>/dev/null || which chromium-browser 2>/dev/null || echo ''").await;
    match result {
        Ok(out) => Ok(!out.trim().is_empty()),
        Err(_) => Ok(false),
    }
}

/// Install Chromium + noVNC in a running sandbox instance
#[tauri::command]
pub async fn install_chromium(
    app: tauri::AppHandle,
    name: String,
) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    // Check if already installed
    let already = backend.exec("which chromium 2>/dev/null || which chromium-browser 2>/dev/null || echo ''").await.unwrap_or_default();
    if !already.trim().is_empty() {
        let _ = app.emit("chromium-install-progress", "Chromium is already installed!");
        return Ok(());
    }

    let _ = app.emit("chromium-install-progress", "Installing Chromium and dependencies (~630MB)...");
    let _ = app.emit("chromium-install-progress", "Note: apk will resume from any previously downloaded packages.");

    // Use exec_with_progress for streaming output
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
    let app2 = app.clone();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            let _ = app2.emit("chromium-install-progress", &line);
        }
    });

    let result = backend.exec_with_progress(
        "sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont 2>&1 || apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont 2>&1",
        &tx,
    ).await;
    drop(tx);

    match result {
        Ok(output) => {
            let _ = app.emit("chromium-install-progress", "✓ Chromium installed successfully!");
            let _ = app.emit("chromium-install-complete", &name);
            emit_instance_changed(&app, InstanceChanged::simple(InstanceAction::InstallChromium, &name));
            tracing::info!("Chromium installed in '{}': {}", name, output.chars().take(200).collect::<String>());
            Ok(())
        }
        Err(e) => {
            let _ = app.emit("chromium-install-progress", &format!("✗ Installation failed: {e}"));
            let _ = app.emit("chromium-install-failed", e.to_string());
            Err(e.to_string())
        }
    }
}

/// Get browser status for a sandbox instance
#[tauri::command]
pub async fn browser_status(name: String) -> Result<serde_json::Value, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let backend_arc: Arc<dyn clawenv_core::sandbox::SandboxBackend> = Arc::from(backend);
    let browser = ChromiumBackend::new(backend_arc);
    let status = browser.status().await.map_err(|e| e.to_string())?;
    serde_json::to_value(&status).map_err(|e| e.to_string())
}

/// Start browser in interactive (noVNC) mode for human intervention
#[tauri::command]
pub async fn browser_start_interactive(name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let vnc_port = inst.browser.vnc_ws_port;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let backend_arc: Arc<dyn clawenv_core::sandbox::SandboxBackend> = Arc::from(backend);
    let browser = ChromiumBackend::new(backend_arc);
    let url = browser.start_interactive(vnc_port).await.map_err(|e| e.to_string())?;
    Ok(url)
}

/// Resume headless mode after human intervention
#[tauri::command]
pub async fn browser_resume_headless(name: String) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let backend_arc: Arc<dyn clawenv_core::sandbox::SandboxBackend> = Arc::from(backend);
    let browser = ChromiumBackend::new(backend_arc);
    browser.resume_headless().await.map_err(|e| e.to_string())
}

/// Notify bridge server that HIL is complete (unblocks hil_request)
#[tauri::command]
pub async fn hil_complete() -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let port = config.config().clawenv.bridge.port;
    let url = format!("http://127.0.0.1:{port}/api/hil/complete");
    reqwest::Client::new().post(&url).send().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Approve a pending exec command
#[tauri::command]
pub async fn exec_approve() -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let port = config.config().clawenv.bridge.port;
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/exec/approve"))
        .send().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Deny a pending exec command
#[tauri::command]
pub async fn exec_deny() -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let port = config.config().clawenv.bridge.port;
    reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/api/exec/deny"))
        .send().await.map_err(|e| e.to_string())?;
    Ok(())
}
