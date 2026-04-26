use clawops_core::config_loader;
use clawops_core::credentials;
use clawops_core::instance::{InstanceRegistry, SandboxKind};
use clawops_core::proxy::{
    apply::{apply_to_sandbox, clear_sandbox_proxy},
    InstanceProxyConfig, InstanceProxyMode, ProxySource, ProxyTriple, Scope,
};
use clawops_core::sandbox_ops::BackendKind;
use clawops_core::wire::{ListResponse, LogResponse, StatusResponse};
use serde::Serialize;
use tauri::{Emitter, Manager, webview::WebviewWindowBuilder};

use crate::claw_meta;
use crate::cli_bridge;
use crate::instance_helper;
use crate::ipc::emit::{emit_instance_changed, InstanceAction, InstanceChanged};
use crate::util;

#[tauri::command]
pub async fn detect_launch_state() -> Result<clawops_core::launcher::LaunchState, String> {
    clawops_core::launcher::detect_launch_state()
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
    /// 0 means "no dashboard; UI lives at gateway_port".
    pub dashboard_port: u16,
}

#[tauri::command]
pub async fn list_instances() -> Result<Vec<InstanceInfo>, String> {
    let data = cli_bridge::run_cli(&["list"]).await.map_err(|e| e.to_string())?;
    let resp: ListResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;

    let instances = resp.instances.into_iter().map(|s| {
        let meta = claw_meta::meta_for(&s.claw);
        InstanceInfo {
            name: s.name,
            // TS-facing field name kept as `claw_type` so existing
            // frontend selectors continue to compile; v2 wire uses
            // `claw`, the rename happens here in the adapter.
            claw_type: s.claw,
            display_name: meta.display_name,
            logo: meta.logo,
            sandbox_type: s.backend,
            sandbox_id: s.sandbox_instance,
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
    let resp: LogResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
    Ok(resp.content)
}

#[tauri::command]
pub async fn open_install_window(app: tauri::AppHandle, instance_name: Option<String>, claw_type: Option<String>) -> Result<(), String> {
    let name = instance_name.unwrap_or_else(|| "default".into());
    let ct = claw_type.unwrap_or_else(|| "openclaw".into());
    let meta = claw_meta::meta_for(&ct);
    let label = format!("install-{name}");
    let url = format!("/index.html?mode=install&name={name}&clawType={ct}");

    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.set_focus();
        return Ok(());
    }

    WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
        .title(format!("Install {} — {name}", meta.display_name))
        .inner_size(900.0, 650.0)
        .resizable(true)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn start_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    refresh_system_proxy_env();
    cli_bridge::run_cli(&["start", &name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::simple(InstanceAction::Start, &name));
    Ok(())
}

/// Re-query the OS system proxy and reinject into this process's env.
/// Native claws inherit env from the GUI; sandbox claws bake proxy at
/// install time so this only affects the GUI->native spawn path. When
/// `[clawenv.proxy]` is enabled the explicit config wins and we leave
/// env alone.
fn refresh_system_proxy_env() {
    if let Ok(global) = config_loader::load_global() {
        if global.proxy.enabled && !global.proxy.http_proxy.is_empty() {
            return;
        }
    }
    if let Some(v) = crate::ipc::detect_system_proxy_native_only() {
        let http  = v.get("http_proxy").and_then(|s| s.as_str()).unwrap_or("");
        let https = v.get("https_proxy").and_then(|s| s.as_str()).unwrap_or("");
        let no_p  = v.get("no_proxy").and_then(|s| s.as_str()).unwrap_or("localhost,127.0.0.1");
        let eh = if http.is_empty()  { https } else { http };
        let es = if https.is_empty() { http }  else { https };
        if !eh.is_empty() {
            let triple = ProxyTriple {
                http: eh.into(),
                https: es.into(),
                no_proxy: no_p.into(),
                source: ProxySource::OsSystem,
            };
            util::apply_proxy_env(&triple);
            return;
        }
    }
    util::clear_proxy_env();
}

#[tauri::command]
pub async fn stop_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["stop", &name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::simple(InstanceAction::Stop, &name));
    Ok(())
}

/// Stop all instances — used by quit dialog. Best-effort: errors per
/// instance are logged but don't abort the loop.
#[tauri::command]
pub async fn stop_all_instances() -> Result<(), String> {
    let registry = InstanceRegistry::with_default_path();
    let instances = registry.list().await.map_err(|e| e.to_string())?;
    for inst in instances {
        if let Err(e) = cli_bridge::run_cli(&["stop", &inst.name]).await {
            tracing::warn!("stop {} failed: {}", inst.name, e);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_instance(app: tauri::AppHandle, name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["uninstall", &name]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(&app, InstanceChanged::deleted(&name));
    Ok(())
}

/// Delete instance with staged progress events for UI dialog.
#[tauri::command]
pub async fn delete_instance_with_progress(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let emit = |stage: &str, status: &str, msg: &str| {
        let _ = app.emit("delete-progress", serde_json::json!({
            "stage": stage, "status": status, "message": msg,
        }));
    };

    let registry = InstanceRegistry::with_default_path();
    let inst = registry.find(&name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Instance '{name}' not found"))?;

    // Stage 1: Stop (best-effort)
    emit("stop", "active", "Stopping instance...");
    let _ = cli_bridge::run_cli(&["stop", &name]).await;
    emit("stop", "done", "Stopped");

    // Stage 2: Kill processes (no-op for sandbox; native uninstall handles this).
    emit("kill", "active", "Killing processes...");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    emit("kill", "done", "Killed");

    // Stage 3: Delete files
    emit("delete_files", "active", "Deleting files...");
    if inst.backend == SandboxKind::Native {
        // Native: cli `uninstall` knows how to clean ~/.clawenv/<name>/.
        if let Err(e) = cli_bridge::run_cli(&["uninstall", &name]).await {
            emit("delete_files", "error", &e.to_string());
            let _ = app.emit("delete-failed", e.to_string());
            return Err(e.to_string());
        }
    } else {
        let backend = instance_helper::backend_for_instance(&inst)?;
        let mut retries = 3;
        loop {
            match backend.destroy().await {
                Ok(_) => { emit("delete_files", "done", "Deleted"); break; }
                Err(e) if retries > 0 => {
                    retries -= 1;
                    emit("delete_files", "active", &format!("Retrying... ({})", e));
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => {
                    emit("delete_files", "error", &e.to_string());
                    let _ = app.emit("delete-failed", e.to_string());
                    return Err(e.to_string());
                }
            }
        }
    }
    emit("delete_files", "done", "Deleted");

    // Stage 4: Update registry
    emit("update_config", "active", "Updating registry...");
    let _ = registry.remove(&name).await;
    emit("update_config", "done", "Done");

    let _ = app.emit("delete-complete", ());
    emit_instance_changed(&app, InstanceChanged::deleted(&name));
    Ok(())
}

#[tauri::command]
pub async fn rename_instance(app: tauri::AppHandle, old_name: String, new_name: String) -> Result<(), String> {
    cli_bridge::run_cli(&["sandbox", "rename", "--from", &old_name, "--to", &new_name])
        .await.map_err(|e| e.to_string())?;
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
    let mut args = vec![
        "--instance".to_string(), name.clone(),
        "sandbox".to_string(), "edit".to_string(),
    ];
    if let Some(c) = cpus { args.extend(["--cpus".into(), c.to_string()]); }
    if let Some(m) = memory_mb { args.extend(["--memory-mb".into(), m.to_string()]); }
    if let Some(d) = disk_gb { args.extend(["--disk-gb".into(), d.to_string()]); }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cli_bridge::run_cli(&refs).await.map_err(|e| e.to_string())?;
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
    let registry = InstanceRegistry::with_default_path();
    let inst = registry.find(&name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("instance `{name}` not in registry"))?;
    let old_gw = instance_helper::gateway_port(&inst);
    let old_tty = instance_helper::ttyd_port(&inst);

    if old_gw != 0 && old_gw != gateway_port {
        let p = old_gw.to_string();
        let _ = cli_bridge::run_cli(&[
            "--instance", &name, "sandbox", "port", "remove", &p,
        ]).await;
    }
    let gw = gateway_port.to_string();
    cli_bridge::run_cli(&[
        "--instance", &name,
        "sandbox", "port", "add", &gw, "3000",
    ]).await.map_err(|e| e.to_string())?;

    if old_tty != 0 && old_tty != ttyd_port {
        let p = old_tty.to_string();
        let _ = cli_bridge::run_cli(&[
            "--instance", &name, "sandbox", "port", "remove", &p,
        ]).await;
    }
    let tty = ttyd_port.to_string();
    cli_bridge::run_cli(&[
        "--instance", &name,
        "sandbox", "port", "add", &tty, "7681",
    ]).await.map_err(|e| e.to_string())?;
    emit_instance_changed(
        &app,
        InstanceChanged::simple(InstanceAction::EditPorts, &name).with_needs_restart(true),
    );
    Ok(())
}

#[tauri::command]
pub async fn get_instance_proxy(name: String) -> Result<serde_json::Value, String> {
    let registry = InstanceRegistry::with_default_path();
    let inst = registry.find(&name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Instance '{name}' not found"))?;
    match &inst.proxy {
        Some(p) => Ok(serde_json::json!({
            "mode": p.mode,
            "http_proxy": p.http_proxy,
            "https_proxy": p.https_proxy,
            "no_proxy": p.no_proxy,
            // Per-instance auth was a v1 field; v2 InstanceProxyConfig
            // doesn't carry username/password — the GUI shows defaults.
            "auth_required": false,
            "auth_user": "",
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
    let registry = InstanceRegistry::with_default_path();
    let mut inst = registry.find(&name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Instance '{name}' not found"))?;

    if inst.backend == SandboxKind::Native {
        return Err(
            "Native mode uses the system proxy only — no per-instance proxy config. \
             Adjust your OS proxy settings and restart the claw.".into()
        );
    }

    let is_inherit = mode == "inherit";
    let auth_required_v = auth_required.unwrap_or(false);
    let auth_user_v = auth_user.unwrap_or_default();
    if auth_required_v && auth_user_v.is_empty() {
        return Err("auth_required=true requires a non-empty auth_user".into());
    }
    if let Some(pw) = auth_password.as_ref().filter(|s| !s.is_empty()) {
        credentials::store_instance_proxy_password(&name, pw)
            .map_err(|e| format!("keychain store: {e}"))?;
    } else if !auth_required_v {
        let _ = credentials::delete_instance_proxy_password(&name);
    }

    let parsed_mode = match mode.as_str() {
        "manual" => InstanceProxyMode::Manual,
        "sync-host" => InstanceProxyMode::SyncHost,
        _ => InstanceProxyMode::None,
    };

    let new_cfg = InstanceProxyConfig {
        mode: parsed_mode,
        http_proxy: http_proxy.clone(),
        https_proxy: https_proxy.clone(),
        no_proxy: no_proxy.clone(),
    };

    inst.proxy = if is_inherit { None } else { Some(new_cfg) };
    registry.update(inst.clone()).await.map_err(|e| e.to_string())?;

    let backend = instance_helper::backend_arc_for_instance(&inst)?;
    let global = config_loader::load_global().map_err(|e| e.to_string())?;
    let backend_kind = match inst.backend {
        SandboxKind::Lima => BackendKind::Lima,
        SandboxKind::Wsl2 => BackendKind::Wsl2,
        SandboxKind::Podman => BackendKind::Podman,
        SandboxKind::Native => unreachable!(),
    };
    let scope = Scope::RuntimeSandbox {
        backend: backend_kind,
        instance: inst.proxy.as_ref(),
    };
    let applied = scope.resolve(&global.proxy, None).await;
    let (http, https) = match applied {
        Some(triple) => {
            apply_to_sandbox(&backend, &triple).await
                .map_err(|e| format!("apply_to_sandbox: {e}"))?;
            (triple.http, triple.https)
        }
        None => {
            let _ = clear_sandbox_proxy(&backend).await;
            (String::new(), String::new())
        }
    };

    emit_instance_changed(
        &app,
        InstanceChanged::simple(InstanceAction::EditPorts, &name).with_needs_restart(true),
    );

    Ok(serde_json::json!({
        "effective_http_proxy": http,
        "effective_https_proxy": https,
        "needs_restart": true,
    }))
}

/// Peek at any proxy config baked into the sandbox (via `/etc/profile.d/proxy.sh`).
/// Returns empty string if no proxy file or instance is native.
#[tauri::command]
pub async fn check_instance_proxy_baked_in(name: String) -> Result<String, String> {
    let registry = InstanceRegistry::with_default_path();
    let inst = registry.find(&name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Instance '{name}' not found"))?;

    if inst.backend == SandboxKind::Native {
        return Ok(String::new());
    }

    let backend = instance_helper::backend_for_instance(&inst)?;
    let out = backend
        .exec("grep -oE 'http_proxy=\"[^\"]*\"' /etc/profile.d/proxy.sh 2>/dev/null | head -1 | sed 's/http_proxy=//;s/\"//g' || true")
        .await
        .unwrap_or_default();
    Ok(out.trim().to_string())
}

#[tauri::command]
pub async fn get_instance_capabilities(name: String) -> Result<serde_json::Value, String> {
    let registry = InstanceRegistry::with_default_path();
    let inst = registry.find(&name).await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Instance '{name}' not found"))?;
    if inst.backend == SandboxKind::Native {
        return Ok(serde_json::json!({
            "rename": false, "resource_edit": false, "port_edit": false,
        }));
    }
    let backend = instance_helper::backend_for_instance(&inst)?;
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
    Ok(resp.summary.health)
}

#[tauri::command]
pub fn exit_app(app: tauri::AppHandle) {
    app.exit(0);
}

