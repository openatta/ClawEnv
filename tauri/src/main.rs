// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bridge_server;
mod claw_meta;
mod cli_bridge;
mod gui_cache;
mod instance_helper;
mod ipc;
mod tray;
mod util;

use clawops_core::browser::{BrowserBackend, BrowserStatus, ChromiumBackend};
use clawops_core::config_loader;
use clawops_core::instance::{InstanceRegistry, SandboxKind};
use clawops_core::launcher;
use clawops_core::proxy::{ProxySource, ProxyTriple};
use clawops_core::wire::StatusResponse;
use tauri::{Emitter, Manager};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Startup proxy injection: feed OS-detected proxy (if any) into this
    // process's env so every spawned subprocess (clawcli, native claws)
    // inherits it. Priority chain: explicit config > shell env > OS detect.
    {
        if let Ok(global) = config_loader::load_global() {
            if global.proxy.enabled && !global.proxy.http_proxy.is_empty() {
                let triple = ProxyTriple {
                    http: global.proxy.http_proxy.clone(),
                    https: if global.proxy.https_proxy.is_empty() {
                        global.proxy.http_proxy.clone()
                    } else {
                        global.proxy.https_proxy.clone()
                    },
                    no_proxy: if global.proxy.no_proxy.is_empty() {
                        "localhost,127.0.0.1".into()
                    } else {
                        global.proxy.no_proxy.clone()
                    },
                    source: ProxySource::GlobalConfig,
                };
                util::apply_proxy_env(&triple);
            }
        }
        if std::env::var("HTTPS_PROXY").ok().filter(|s| !s.is_empty()).is_none()
            && std::env::var("https_proxy").ok().filter(|s| !s.is_empty()).is_none()
        {
            if let Some(v) = ipc::detect_system_proxy_native_only() {
                let http  = v.get("http_proxy").and_then(|s| s.as_str()).unwrap_or("");
                let https = v.get("https_proxy").and_then(|s| s.as_str()).unwrap_or("");
                let no_p  = v.get("no_proxy").and_then(|s| s.as_str()).unwrap_or("localhost,127.0.0.1");
                if !http.is_empty() || !https.is_empty() {
                    let effective_http  = if http.is_empty() { https } else { http };
                    let effective_https = if https.is_empty() { http } else { https };
                    let triple = ProxyTriple {
                        http: effective_http.to_string(),
                        https: effective_https.to_string(),
                        no_proxy: no_p.to_string(),
                        source: ProxySource::OsSystem,
                    };
                    util::apply_proxy_env(&triple);
                    tracing::info!(target: "clawenv::proxy", "startup: injected OS proxy {effective_http}");
                }
            }
        }
    }

    // Pin LIMA_HOME to ~/.clawenv/lima so every spawned limactl uses our
    // private data dir instead of the system default ~/.lima.
    #[cfg(target_os = "macos")]
    util::init_lima_env();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            tray::setup_tray(app.handle())?;

            // OS proxy watcher — 30s polling, emits `os-proxy-changed` on
            // transitions. Polling is the same on macOS + Windows without
            // platform-specific runloop gymnastics.
            {
                let watcher_handle = app.handle().clone();
                std::thread::spawn(move || {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut last_sig: u64 = 0;
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(30));
                        let current = ipc::detect_system_proxy_native_only();
                        let mut h = DefaultHasher::new();
                        let payload = current.as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        payload.hash(&mut h);
                        let sig = h.finish();
                        if sig != last_sig {
                            last_sig = sig;
                            if let Some(v) = current.as_ref() {
                                let http  = v.get("http_proxy").and_then(|s| s.as_str()).unwrap_or("");
                                let https = v.get("https_proxy").and_then(|s| s.as_str()).unwrap_or("");
                                let no_p  = v.get("no_proxy").and_then(|s| s.as_str()).unwrap_or("localhost,127.0.0.1");
                                let eh = if http.is_empty() { https } else { http };
                                let es = if https.is_empty() { http } else { https };
                                if !eh.is_empty() {
                                    let t = ProxyTriple {
                                        http: eh.into(),
                                        https: es.into(),
                                        no_proxy: no_p.into(),
                                        source: ProxySource::OsSystem,
                                    };
                                    util::apply_proxy_env(&t);
                                }
                            } else {
                                util::clear_proxy_env();
                            }
                            let _ = watcher_handle.emit("os-proxy-changed", &current);
                            tracing::info!(target: "clawenv::proxy", "OS proxy changed (watcher)");
                        }
                    }
                });
            }

            // If launched with --minimized (autostart), hide main window
            if std::env::args().any(|a| a == "--minimized") {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }

            // Detect launch state and emit to frontend
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = launcher::detect_launch_state().await;
                tracing::info!("Launch state: {:?}", state);
                if let Ok(state) = state {
                    let _ = handle.emit("launch-state", &state);
                }
            });

            // Embedded HIL + exec-approval bridge HTTP server. Lifted from
            // v1 core; long-term home is the AttaRun bridge daemon
            // (docs/v2/v0.5.x-features.md "Bridge admin UI").
            let bridge_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(global) = config_loader::load_global() {
                    if global.bridge.enabled {
                        tracing::info!("Starting bridge server on port {}", global.bridge.port);
                        let permissions = parse_bridge_permissions(&global.bridge.permissions);
                        let bh = bridge_handle.clone();
                        let emitter: bridge_server::EventEmitter =
                            Box::new(move |event, payload| {
                                let _ = bh.emit(event, payload.to_string());
                            });
                        let hw_token = std::env::var("CLAWENV_HW_TOKEN").unwrap_or_default();

                        // Build the local MCP server state (input/screen
                        // tools for agents to drive the host) and write
                        // the discovery descriptor. The registry is
                        // empty on Linux per CLAUDE.md, so this is a
                        // no-op there other than the descriptor file.
                        let mcp_state = build_mcp_state(global.bridge.port);

                        if let Err(e) = bridge_server::start_bridge(
                            global.bridge.port,
                            permissions,
                            Some(emitter),
                            hw_token,
                            mcp_state,
                        ).await {
                            tracing::error!("Bridge server failed: {e}");
                        }
                    }
                }
            });

            // Background instance health monitor — polls clawcli `status`.
            let monitor_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use std::collections::HashMap;

                tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                let interval: u32 = config_loader::load_global()
                    .ok()
                    .map(|_| 5_u32)
                    .unwrap_or(5);

                let mut prev_health: HashMap<String, String> = HashMap::new();

                loop {
                    let registry = InstanceRegistry::with_default_path();
                    if let Ok(instances) = registry.list().await {
                        for inst in &instances {
                            let health = match cli_bridge::run_cli(&["status", &inst.name]).await {
                                Ok(data) => {
                                    serde_json::from_value::<StatusResponse>(data)
                                        .map(|r| r.summary.health)
                                        .unwrap_or_else(|_| "unreachable".into())
                                }
                                Err(_) => "unreachable".into(),
                            };

                            let _ = monitor_handle.emit("instance-health", serde_json::json!({
                                "instance_name": inst.name,
                                "health": health,
                            }));

                            if let Some(prev) = prev_health.get(&inst.name) {
                                if *prev != health {
                                    let _ = tray::refresh_tray(&monitor_handle);
                                    let (title, body) = match health.as_str() {
                                        "running" => ("Instance Recovered", format!("'{}' is now running", inst.name)),
                                        "stopped" => ("Instance Stopped", format!("'{}' has stopped", inst.name)),
                                        _ => ("Instance Unreachable", format!("'{}' is unreachable", inst.name)),
                                    };
                                    tray::send_notification(&monitor_handle, title, &body);
                                    let tray_status = match health.as_str() {
                                        "running" => tray::TrayStatus::Running,
                                        "stopped" => tray::TrayStatus::Stopped,
                                        _ => tray::TrayStatus::Error,
                                    };
                                    tray::set_tray_status(&monitor_handle, tray_status);
                                }
                            }
                            prev_health.insert(inst.name.clone(), health);

                            // Browser HIL probe — only for sandbox claws with browser enabled.
                            if inst.backend != SandboxKind::Native && inst.browser.enabled {
                                if let Ok(backend_arc) = instance_helper::backend_arc_for_instance(inst) {
                                    let browser = ChromiumBackend::new(backend_arc);
                                    if let Ok(BrowserStatus::Interactive { ref novnc_url })
                                        = browser.status().await
                                    {
                                        let _ = monitor_handle.emit("hil-required", serde_json::json!({
                                            "instance": inst.name,
                                            "novnc_url": novnc_url,
                                        }));
                                    }
                                }
                            }
                        }
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(interval as u64)).await;
                }
            });

            // Background update check — refresh GUI cache for tray badge.
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                loop {
                    let registry = InstanceRegistry::with_default_path();
                    if let Ok(instances) = registry.list().await {
                        let npm_registry = config_loader::load_global()
                            .ok()
                            .map(|g| g.mirrors.npm_registry.clone())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| "https://registry.npmjs.org".into());
                        for inst in &instances {
                            let meta = claw_meta::meta_for(&inst.claw);
                            if meta.npm_package.is_empty() || inst.claw_version.is_empty() {
                                continue;
                            }
                            match clawops_core::update::check_latest_version(
                                &inst.claw_version,
                                &npm_registry,
                                meta.npm_package,
                            ).await {
                                Ok(info) => {
                                    gui_cache::record_latest(&inst.name, &info.latest);
                                    if info.has_upgrade {
                                        tracing::info!(
                                            "Update available for '{}': {} → {}",
                                            inst.name, info.current, info.latest
                                        );
                                        let title = if info.is_security_release {
                                            "Security Update Available"
                                        } else {
                                            "Update Available"
                                        };
                                        tray::send_notification(
                                            &update_handle,
                                            title,
                                            &format!("{} {} → {} for '{}'",
                                                meta.display_name, info.current, info.latest, inst.name),
                                        );
                                    }
                                }
                                Err(e) => tracing::debug!("Update check failed for '{}': {e}", inst.name),
                            }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::detect_launch_state,
            ipc::list_instances,
            ipc::get_instance_logs,
            ipc::install_openclaw,
            ipc::start_instance,
            ipc::stop_instance,
            ipc::stop_all_instances,
            ipc::delete_instance,
            ipc::delete_instance_with_progress,
            ipc::rename_instance,
            ipc::edit_instance_resources,
            ipc::edit_instance_ports,
            ipc::get_instance_capabilities,
            ipc::get_instance_proxy,
            ipc::set_instance_proxy,
            ipc::check_instance_proxy_baked_in,
            ipc::test_instance_network,
            ipc::open_install_window,
            ipc::get_instance_health,
            ipc::save_settings,
            ipc::autostart_is_enabled,
            ipc::autostart_set,
            ipc::diagnose_instances,
            ipc::fix_diagnostic_issue,
            ipc::test_connectivity,
            ipc::detect_system_proxy,
            ipc::system_check,
            ipc::install_prerequisites,
            ipc::pick_import_file,
            ipc::validate_import_file,
            ipc::has_native_instance,
            ipc::list_sandbox_vms,
            ipc::get_sandbox_disk_usage,
            ipc::sandbox_vm_action,
            ipc::check_chromium_installed,
            ipc::install_chromium,
            ipc::browser_status,
            ipc::browser_start_interactive,
            ipc::browser_resume_headless,
            ipc::hil_complete,
            ipc::exec_approve,
            ipc::exec_deny,
            ipc::get_gateway_token,
            ipc::get_bridge_config,
            ipc::save_bridge_config,
            ipc::open_url_in_browser,
            ipc::create_default_config,
            ipc::check_instance_update,
            ipc::upgrade_instance,
            ipc::claw::list_claw_types,
            ipc::restart_computer,
            ipc::export_sandbox,
            ipc::export_native_bundle,
            ipc::export_cancel,
            ipc::lite::lite_scan_packages,
            ipc::lite::pick_import_folder,
            ipc::mcp_perm_status,
            ipc::mcp_open_perm_url,
            ipc::exit_app,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building ClawEnv")
        .run(|app, event| {
            #[cfg(not(target_os = "macos"))]
            let _ = (&app, &event);

            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { has_visible_windows, .. } = event {
                if !has_visible_windows {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
            }
        });
}

/// Build the optional MCP state for the local input/screen tool
/// server. Generates a fresh per-launch token, populates the platform
/// tool registry, and writes `~/.clawenv/bridge.mcp.json` so claws and
/// other MCP clients can discover the endpoint without copy-paste.
///
/// Returns `None` when the descriptor write fails — the bridge still
/// starts, just without MCP. We don't bail the whole bridge on this
/// path because HIL + exec-approval are independently valuable.
fn build_mcp_state(bridge_port: u16) -> Option<bridge_server::McpState> {
    use bridge_server::mcp::{
        default_descriptor_path, random_token, write_descriptor, BridgeMcpDescriptor,
    };
    // Cap screenshot output dimension; a single 4K capture base64-encoded
    // is ~10MB which is fine over loopback but we don't need to ship 4K
    // by default. Override via env later if a claw insists.
    let registry = clawops_core::input::build_default(1920);
    let token = random_token();
    let url = format!("http://127.0.0.1:{bridge_port}/mcp");
    let desc = BridgeMcpDescriptor {
        url: url.clone(),
        token: token.clone(),
        pid: std::process::id(),
    };
    let path = default_descriptor_path();
    if let Err(e) = write_descriptor(&path, &desc) {
        tracing::warn!("MCP descriptor write failed ({}): {e}", path.display());
        return None;
    }
    tracing::info!("MCP server descriptor written to {}", path.display());
    Some(bridge_server::McpState { registry, token })
}

/// Translate the toml::Table stored under `[clawenv.bridge.permissions]`
/// into the strongly-typed `BridgePermissions` the bridge server
/// consumes. Falls back to defaults on parse failure so a malformed
/// permissions block doesn't crash the GUI.
fn parse_bridge_permissions(t: &toml::Table) -> bridge_server::BridgePermissions {
    serde_json::to_value(t)
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_else(|| {
            // Minimal safe defaults — file ops disabled, exec gated.
            bridge_server::BridgePermissions {
                file_read: vec![],
                file_write: vec![],
                file_deny: vec![],
                exec_allow: vec![],
                exec_deny: vec![],
                require_approval: vec!["**".into()],
                auto_approve: vec![],
                shell_enabled: false,
                shell_program: if cfg!(target_os = "windows") { "powershell".into() } else { "bash".into() },
                shell_require_approval: true,
            }
        })
}
