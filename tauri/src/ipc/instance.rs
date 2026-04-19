use clawenv_core::api::{ListResponse, StatusResponse};
use clawenv_core::claw::ClawRegistry;
use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
use serde::Serialize;
use tauri::{Emitter, Manager, webview::WebviewWindowBuilder};

use crate::cli_bridge;
use crate::ipc::emit::{emit_instance_changed, InstanceAction, InstanceChanged};

#[tauri::command]
pub async fn detect_launch_state() -> Result<clawenv_core::launcher::LaunchState, String> {
    clawenv_core::launcher::detect_launch_state()
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct InstanceInfo {
    pub name: String,
    pub claw_type: String,
    pub display_name: String,
    pub logo: String,
    pub sandbox_type: String,
    /// Sandbox VM id (`clawenv-<hash>` for managed VMs, `"native"` for
    /// native installs). Surfaces in the ClawPage info table so users
    /// can correlate claws ↔ VM cards on SandboxPage.
    pub sandbox_id: String,
    pub version: String,
    pub gateway_port: u16,
    pub ttyd_port: u16,
    /// Dashboard port for claws that split UI from gateway (Hermes).
    /// 0 means "no dashboard; UI lives at gateway_port". Forwarded here
    /// straight from `InstanceSummary::dashboard_port`; dropping this
    /// field would force the frontend to fall back to gateway_port,
    /// which is exactly the bug that made the Hermes "Open Control
    /// Panel" button land on an empty page before v0.2.7.
    pub dashboard_port: u16,
}

#[tauri::command]
pub async fn list_instances() -> Result<Vec<InstanceInfo>, String> {
    let data = cli_bridge::run_cli(&["list"]).await.map_err(|e| e.to_string())?;
    let resp: ListResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;

    let registry = ClawRegistry::load();
    let instances = resp.instances.into_iter().map(|s| {
        let desc = registry.get(&s.claw_type);
        InstanceInfo {
            name: s.name,
            claw_type: s.claw_type,
            display_name: desc.display_name.clone(),
            logo: desc.logo.clone(),
            sandbox_type: s.sandbox_type,
            sandbox_id: s.sandbox_id,
            version: s.version,
            gateway_port: s.gateway_port,
            ttyd_port: s.ttyd_port,
            dashboard_port: s.dashboard_port,
        }
    }).collect();

    Ok(instances)
}

#[tauri::command]
pub async fn get_instance_logs(name: String) -> Result<String, String> {
    let data = cli_bridge::run_cli(&["logs", &name]).await.map_err(|e| e.to_string())?;
    Ok(data.as_str().unwrap_or("").to_string())
}

#[tauri::command]
pub async fn open_install_window(app: tauri::AppHandle, instance_name: Option<String>, claw_type: Option<String>) -> Result<(), String> {
    let name = instance_name.unwrap_or_else(|| "default".into());
    let ct = claw_type.unwrap_or_else(|| "openclaw".into());
    let registry = ClawRegistry::load();
    let desc = registry.get(&ct);
    let label = format!("install-{name}");
    let url = format!("/index.html?mode=install&name={name}&clawType={ct}");

    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
        .title(format!("Install {} — {name}", desc.display_name))
        .inner_size(900.0, 650.0)
        .resizable(true)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn start_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    // Refresh the GUI process's env from the OS system proxy before spawning
    // the CLI (which spawns the native claw). Covers the case where the user
    // toggled Clash / changed System Preferences AFTER ClawEnv started — the
    // original startup-time injection would be stale, and a Native claw
    // would see the old proxy. Sandbox instances don't strictly need this
    // (their proxy.sh is written per-instance), but it's a cheap call and
    // keeps behavior uniform.
    refresh_system_proxy_env();
    cli_bridge::run_cli(&["start", &name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::simple(InstanceAction::Start, &name));
    Ok(())
}

/// Re-query the OS system proxy and reinject into this process's env
/// using the unified `proxy_resolver`. Called on every start_instance
/// so Native claws see fresh OS proxy if the user toggled Clash between
/// app launch and claw start. When `config.toml.proxy.enabled` is true
/// the explicit config wins and we leave env alone.
fn refresh_system_proxy_env() {
    use clawenv_core::config::{proxy_resolver, ConfigManager};
    if let Ok(config) = ConfigManager::load() {
        if config.config().clawenv.proxy.enabled
            && !config.config().clawenv.proxy.http_proxy.is_empty()
        {
            return; // explicit config wins
        }
    }
    if let Some(v) = crate::ipc::detect_system_proxy_native_only() {
        let http  = v.get("http_proxy").and_then(|s| s.as_str()).unwrap_or("");
        let https = v.get("https_proxy").and_then(|s| s.as_str()).unwrap_or("");
        let no_p  = v.get("no_proxy").and_then(|s| s.as_str()).unwrap_or("localhost,127.0.0.1");
        let eh = if http.is_empty()  { https } else { http };
        let es = if https.is_empty() { http }  else { https };
        if !eh.is_empty() {
            let triple = proxy_resolver::ProxyTriple {
                http: eh.into(),
                https: es.into(),
                no_proxy: no_p.into(),
                source: proxy_resolver::ProxySource::OsSystem,
            };
            proxy_resolver::apply_env(&triple);
            return;
        }
    }
    // OS reports no proxy → clear (stale values shouldn't stick).
    proxy_resolver::clear_env();
}

#[tauri::command]
pub async fn stop_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["stop", &name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::simple(InstanceAction::Stop, &name));
    Ok(())
}

/// Stop all instances — used by quit dialog
#[tauri::command]
pub async fn stop_all_instances() -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    for inst in config.instances() {
        let _ = clawenv_core::manager::instance::stop_instance(inst).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["uninstall", "--name", &name]).await.map_err(|e| e.to_string())?;
    // `instance-changed` is the canonical state-sync event; the legacy
    // `instances-changed` (plural) is no longer emitted — MainLayout converged
    // its refresh logic onto `instance-changed`.
    emit_instance_changed(&app, InstanceChanged::deleted(&name));
    Ok(())
}

/// Delete instance with staged progress events for UI dialog
#[tauri::command]
pub async fn delete_instance_with_progress(app: tauri::AppHandle, name: String) -> Result<(), String> {
    use clawenv_core::manager::instance;
    use clawenv_core::sandbox::SandboxType;

    let emit = |stage: &str, status: &str, msg: &str| {
        let _ = app.emit("delete-progress", serde_json::json!({
            "stage": stage, "status": status, "message": msg,
        }));
    };

    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?.clone();

    // Stage 1: Stop
    emit("stop", "active", "Stopping instance...");
    let _ = instance::stop_instance(&inst).await;
    emit("stop", "done", "Stopped");

    // Stage 2: Kill processes
    emit("kill", "active", "Killing processes...");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    if inst.sandbox_type == SandboxType::Native {
        instance::kill_native_gateway_public(inst.gateway.gateway_port).await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    emit("kill", "done", "Killed");

    // Stage 3: Delete files
    emit("delete_files", "active", "Deleting files...");
    let backend = instance::backend_for_instance(&inst).map_err(|e| e.to_string())?;
    let mut retries = 3;
    loop {
        match backend.destroy().await {
            Ok(_) => { emit("delete_files", "done", "Deleted"); break; }
            Err(e) if retries > 0 => {
                retries -= 1;
                emit("delete_files", "active", &format!("Retrying... ({})", e));
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                // Kill again in case something respawned
                if inst.sandbox_type == SandboxType::Native {
                    instance::kill_native_gateway_public(inst.gateway.gateway_port).await;
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
            Err(e) => {
                emit("delete_files", "error", &e.to_string());
                let _ = app.emit("delete-failed", e.to_string());
                return Err(e.to_string());
            }
        }
    }

    // Stage 4: Update config
    emit("update_config", "active", "Updating config...");
    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;
    config.config_mut().instances.retain(|i| i.name != name);
    config.save().map_err(|e| e.to_string())?;
    emit("update_config", "done", "Done");

    let _ = app.emit("delete-complete", ());
    // Single canonical event for state sync. DeleteProgress.tsx already got
    // its own `delete-complete` above; `instance-changed` drives list/tab/
    // health refresh in MainLayout.
    emit_instance_changed(&app, InstanceChanged::deleted(&name));
    Ok(())
}

#[tauri::command]
pub async fn rename_instance(app: tauri::AppHandle, old_name: String, new_name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["rename", &old_name, &new_name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::renamed(&old_name, &new_name));
    Ok(())
}

#[tauri::command]
pub async fn edit_instance_resources(
    app: tauri::AppHandle,
    name: String,
    cpus: Option<u32>,
    memory_mb: Option<u32>,
    disk_gb: Option<u32>,
) -> Result<(), String> {
    let mut args = vec!["edit".to_string(), name.clone()];
    if let Some(c) = cpus { args.extend(["--cpus".into(), c.to_string()]); }
    if let Some(m) = memory_mb { args.extend(["--memory".into(), m.to_string()]); }
    if let Some(d) = disk_gb { args.extend(["--disk".into(), d.to_string()]); }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cli_bridge::run_cli(&refs).await.map_err(|e| e.to_string())?;
    // Changes only take effect after the VM/process restarts — surface that to the user.
    emit_instance_changed(
        &app,
        InstanceChanged::simple(InstanceAction::EditResources, &name).with_needs_restart(true),
    );
    Ok(())
}

#[tauri::command]
pub async fn edit_instance_ports(
    app: tauri::AppHandle,
    name: String,
    gateway_port: u16,
    ttyd_port: u16,
) -> Result<(), String> {
    cli_bridge::run_cli(&[
        "edit", &name,
        "--gateway-port", &gateway_port.to_string(),
        "--ttyd-port", &ttyd_port.to_string(),
    ]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(
        &app,
        InstanceChanged::simple(InstanceAction::EditPorts, &name).with_needs_restart(true),
    );
    Ok(())
}

/// Per-instance proxy config for the ClawPage proxy modal. `None` (default
/// on a fresh instance) means "inherit global proxy". Setting to
/// `mode="none"` explicitly disables proxy for just this instance.
#[tauri::command]
pub async fn get_instance_proxy(name: String) -> Result<serde_json::Value, String> {
    use clawenv_core::config::ConfigManager;
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = config.instances().iter()
        .find(|i| i.name == name)
        .ok_or_else(|| format!("Instance '{name}' not found"))?;
    match &inst.proxy {
        Some(p) => Ok(serde_json::json!({
            "mode": p.mode,
            "http_proxy": p.http_proxy,
            "https_proxy": p.https_proxy,
            "no_proxy": p.no_proxy,
            "auth_required": p.auth_required,
            "auth_user": p.auth_user,
            // Password is never returned — users re-enter it if they want
            // to change it. Current value stays in keychain.
        })),
        None => Ok(serde_json::json!({
            "mode": "inherit",
            "http_proxy": "",
            "https_proxy": "",
            "no_proxy": "",
            "auth_required": false,
            "auth_user": "",
        })),
    }
}

/// Save + apply a new proxy config for this instance. If the sandbox is
/// running (Lima/Podman/WSL), rewrites `/etc/profile.d/proxy.sh` and npm
/// config in-place. Claws already-running don't pick up the change until
/// they restart — the caller is expected to prompt the user for a restart
/// when `needs_restart == true`.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn set_instance_proxy(
    app: tauri::AppHandle,
    name: String,
    mode: String,
    http_proxy: String,
    https_proxy: String,
    no_proxy: String,
    auth_required: Option<bool>,
    auth_user: Option<String>,
    auth_password: Option<String>,
) -> Result<serde_json::Value, String> {
    use clawenv_core::config::{keychain, ConfigManager, InstanceProxyConfig};
    use clawenv_core::config::proxy_resolver::{self, Scope};

    let mut config = ConfigManager::load().map_err(|e| e.to_string())?;

    // Pull out owned copies we need after the mutable borrow ends.
    let (sandbox_type, backend_res) = {
        let inst = config.instances().iter()
            .find(|i| i.name == name)
            .ok_or_else(|| format!("Instance '{name}' not found"))?;
        (inst.sandbox_type, instance::backend_for_instance(inst))
    };

    // Native mode is system-proxy-only by design — the native claw inherits
    // the GUI process's env (which already carries the system proxy, injected
    // at Tauri startup). Per-instance proxy overrides would diverge from that
    // contract and confuse users, so we reject any attempt to set one.
    if sandbox_type == clawenv_core::sandbox::SandboxType::Native {
        return Err(
            "Native mode uses the system proxy only — no per-instance proxy config. Adjust your OS proxy settings (System Preferences / Internet Options) and restart the claw."
            .into()
        );
    }

    // "inherit" is a sentinel — we clear the per-instance override and let
    // the install-time global proxy apply as before. Everything else gets
    // persisted (including "none" as an explicit opt-out for this instance).
    let is_inherit = mode == "inherit";
    let auth_required_v = auth_required.unwrap_or(false);
    let auth_user_v = auth_user.unwrap_or_default();

    // Persist password to keychain BEFORE config write so the resolver's
    // first `resolve` call (inside apply loop) can find it. Delete entry
    // when auth is turned off to avoid stale credentials.
    if auth_required_v && auth_user_v.is_empty() {
        return Err("auth_required=true requires a non-empty auth_user".into());
    }
    if let Some(pw) = auth_password.as_ref().filter(|s| !s.is_empty()) {
        keychain::store_instance_proxy_password(&name, pw)
            .map_err(|e| format!("keychain store: {e}"))?;
    } else if !auth_required_v {
        // Best-effort cleanup — ignore errors (no prior entry is fine).
        let _ = keychain::delete_instance_proxy_password(&name);
    }

    let new_cfg = InstanceProxyConfig {
        mode: if is_inherit { "none".into() } else { mode.clone() },
        http_proxy: http_proxy.clone(),
        https_proxy: https_proxy.clone(),
        no_proxy: no_proxy.clone(),
        auth_required: auth_required_v,
        auth_user: auth_user_v,
    };

    // Write config FIRST so the resolver sees the new value, then resolve
    // + apply via the unified path. This ensures the URL written into the
    // VM is the exact one the resolver would return — no divergence risk.
    config.update_instance(&name, |i| {
        if is_inherit {
            i.proxy = None;
        } else {
            i.proxy = Some(new_cfg.clone());
        }
    }).map_err(|e| e.to_string())?;

    let backend = backend_res.map_err(|e| e.to_string())?;
    // Re-read the updated instance for the resolve call.
    let inst_owned = config.instances().iter()
        .find(|i| i.name == name)
        .cloned()
        .ok_or_else(|| format!("Instance '{name}' vanished after save"))?;
    let scope = Scope::RuntimeSandbox {
        instance: &inst_owned,
        backend: &*backend,
    };
    let applied = match scope.resolve(&config).await {
        Some(triple) => {
            proxy_resolver::apply_to_sandbox(&triple, &*backend).await
                .map_err(|e| format!("apply_to_sandbox: {e}"))?;
            Some(triple)
        }
        None => {
            proxy_resolver::clear_sandbox(&*backend).await.ok();
            None
        }
    };

    emit_instance_changed(
        &app,
        InstanceChanged::simple(InstanceAction::EditPorts, &name).with_needs_restart(true),
    );

    let (http, https) = match applied {
        Some(t) => (t.http, t.https),
        None => (String::new(), String::new()),
    };
    Ok(serde_json::json!({
        "effective_http_proxy": http,
        "effective_https_proxy": https,
        "needs_restart": true,
    }))
}

/// Peek at any proxy config baked into the sandbox (via `/etc/profile.d/proxy.sh`).
/// Used after import to warn the user about stale proxy from the source machine.
/// Returns empty string if no proxy file or instance is native.
#[tauri::command]
pub async fn check_instance_proxy_baked_in(name: String) -> Result<String, String> {
    use clawenv_core::config::ConfigManager;
    use clawenv_core::sandbox::SandboxType;

    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = config.instances().iter()
        .find(|i| i.name == name)
        .ok_or_else(|| format!("Instance '{name}' not found"))?;

    if inst.sandbox_type == SandboxType::Native {
        return Ok(String::new());
    }

    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    // `|| true` swallows errors so an offline VM just reports "no bake-in".
    let out = backend
        .exec("grep -oE 'http_proxy=\"[^\"]*\"' /etc/profile.d/proxy.sh 2>/dev/null | head -1 | sed 's/http_proxy=//;s/\"//g' || true")
        .await
        .unwrap_or_default();
    Ok(out.trim().to_string())
}

#[tauri::command]
pub async fn get_instance_capabilities(name: String) -> Result<serde_json::Value, String> {
    // Capabilities are backend-specific — keep direct core call (lightweight, no subprocess needed)
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "rename": backend.supports_rename(),
        "resource_edit": backend.supports_resource_edit(),
        "port_edit": backend.supports_port_edit(),
    }))
}

#[tauri::command]
pub async fn get_instance_health(name: String) -> Result<String, String> {
    let data = cli_bridge::run_cli(&["status", &name]).await.map_err(|e| e.to_string())?;
    let resp: StatusResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp.health)
}

#[tauri::command]
pub fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}
