// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli_bridge;
mod tray;
mod ipc;

use clawenv_core::browser::BrowserBackend;
use clawenv_core::config::ConfigManager;
use clawenv_core::launcher;
use tauri::{Emitter, Manager};

fn main() {
    // Initialize logging — visible when run from terminal
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Startup proxy injection: feed OS-detected proxy (if any) into this
    // process's env so every spawned subprocess (clawcli, native claws)
    // inherits it. `core::proxy_resolver` handles priority chain —
    // shell env > config > OS detect. See docs/23-proxy-architecture.md §3.
    //
    // OS detection itself lives in `ipc::detect_system_proxy_native_only`
    // because it pulls in GUI-only deps (system-configuration / winreg /
    // gsettings). We pass the detected payload into env manually here so
    // when the CLI subprocess's resolver later queries env it sees it.
    {
        use clawenv_core::config::{proxy_resolver, ConfigManager};
        // First, explicit config wins.
        if let Ok(config) = ConfigManager::load() {
            if let Some(t) = proxy_resolver::triple_from_config_proxy(
                &config.config().clawenv.proxy,
                proxy_resolver::ProxySource::GlobalConfig,
            ) {
                proxy_resolver::apply_env(&t);
            }
        }
        // Then OS detection fills gaps.
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
                    let triple = proxy_resolver::ProxyTriple {
                        http: effective_http.to_string(),
                        https: effective_https.to_string(),
                        no_proxy: no_p.to_string(),
                        source: proxy_resolver::ProxySource::OsSystem,
                    };
                    proxy_resolver::apply_env(&triple);
                    tracing::info!(target: "clawenv::proxy", "startup: injected OS proxy {effective_http}");
                }
            }
        }
    }

    // Pin LIMA_HOME to ~/.clawenv/lima so every spawned limactl uses the
    // private data directory instead of the system default ~/.lima.
    #[cfg(target_os = "macos")]
    clawenv_core::sandbox::init_lima_env();

    // Pin Podman's XDG_DATA_HOME / XDG_RUNTIME_DIR to ~/.clawenv/podman-*
    // so container storage, volumes, db and the runtime socket all live
    // inside our tree — parity with Lima (macOS) and WSL (Windows, which
    // already imports distros under ~/.clawenv/wsl/).
    #[cfg(target_os = "linux")]
    clawenv_core::sandbox::init_podman_env();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            // Initialize system tray
            tray::setup_tray(app.handle())?;

            // OS proxy watcher — 30s polling, emits `os-proxy-changed` on
            // transitions. Simple polling trumps per-platform notification
            // APIs because it works the same on macOS + Windows without
            // platform-specific runloop gymnastics. Polling interval is the
            // worst-case latency users see between "toggle Clash" and "claw
            // sees new proxy", but Restart instance is free anyway so this
            // is mostly for the UI indicator and passive env refresh.
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
                            // Refresh env for newly-spawned subprocesses.
                            if let Some(v) = current.as_ref() {
                                let http  = v.get("http_proxy").and_then(|s| s.as_str()).unwrap_or("");
                                let https = v.get("https_proxy").and_then(|s| s.as_str()).unwrap_or("");
                                let no_p  = v.get("no_proxy").and_then(|s| s.as_str()).unwrap_or("localhost,127.0.0.1");
                                let eh = if http.is_empty() { https } else { http };
                                let es = if https.is_empty() { http } else { https };
                                if !eh.is_empty() {
                                    let t = clawenv_core::config::proxy_resolver::ProxyTriple {
                                        http: eh.into(),
                                        https: es.into(),
                                        no_proxy: no_p.into(),
                                                        source: clawenv_core::config::proxy_resolver::ProxySource::OsSystem,
                                    };
                                    clawenv_core::config::proxy_resolver::apply_env(&t);
                                }
                            } else {
                                clawenv_core::config::proxy_resolver::clear_env();
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

            // Start bridge server if enabled
            let bridge_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(config) = ConfigManager::load() {
                    let bridge_cfg = &config.config().clawenv.bridge;
                    if bridge_cfg.enabled {
                        tracing::info!("Starting bridge server on port {}", bridge_cfg.port);
                        // Create event emitter closure for HIL notifications
                        let bh = bridge_handle.clone();
                        let emitter: clawenv_core::bridge::server::EventEmitter =
                            Box::new(move |event, payload| {
                                let _ = bh.emit(event, payload.to_string());
                            });
                        let hw_token = std::env::var("CLAWENV_HW_TOKEN").unwrap_or_default();
                        if let Err(e) = clawenv_core::bridge::server::start_bridge(
                            bridge_cfg.port,
                            bridge_cfg.permissions.clone(),
                            Some(emitter),
                            hw_token,
                        ).await {
                            tracing::error!("Bridge server failed: {e}");
                        }
                    }
                }
            });

            // Spawn background instance health monitor — polls CLI `status` command
            let monitor_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use std::collections::HashMap;
                use clawenv_core::api::StatusResponse;

                tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                let interval = ConfigManager::load()
                    .map(|c| c.config().clawenv.tray.monitor_interval_sec)
                    .unwrap_or(5);

                let mut prev_health: HashMap<String, String> = HashMap::new();

                loop {
                    // Reload config each cycle to pick up new/removed instances
                    if let Ok(config) = ConfigManager::load() {
                        for inst in config.instances() {
                            let health = match cli_bridge::run_cli(&["status", &inst.name]).await {
                                Ok(data) => {
                                    serde_json::from_value::<StatusResponse>(data)
                                        .map(|r| r.health)
                                        .unwrap_or_else(|_| "unreachable".into())
                                }
                                Err(_) => "unreachable".into(),
                            };

                            // Emit health event to frontend
                            let _ = monitor_handle.emit("instance-health", serde_json::json!({
                                "instance_name": inst.name,
                                "health": health,
                            }));

                            // Notify on health changes
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

                            // Check browser HIL status for sandbox instances
                            if inst.sandbox_type != clawenv_core::sandbox::SandboxType::Native
                                && inst.browser.enabled
                            {
                                if let Ok(backend) = clawenv_core::manager::instance::backend_for_instance(inst) {
                                    let browser = clawenv_core::browser::chromium::ChromiumBackend::new(
                                        std::sync::Arc::from(backend) as std::sync::Arc<dyn clawenv_core::sandbox::SandboxBackend>
                                    );
                                    if let Ok(clawenv_core::browser::BrowserStatus::Interactive { ref novnc_url })
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

            // Background update check — cache results silently, no popup.
            // User can check updates manually from ClawPage.
            let update_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Check network periodically
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                loop {
                    if let Ok(mut config) = ConfigManager::load() {
                        let npm_registry = config.config().clawenv.mirrors.npm_registry_url();
                        let instances = config.instances().to_vec();
                        for inst in &instances {
                            let claw_reg = clawenv_core::claw::ClawRegistry::load();
                            let npm_pkg = claw_reg.get(&inst.claw_type).npm_package.clone();
                            match clawenv_core::update::checker::check_latest_version(&inst.version, &npm_registry, &npm_pkg).await {
                                Ok(info) => {
                                    // Cache the result
                                    if let Some(entry) = config.config_mut().instances.iter_mut().find(|i| i.name == inst.name) {
                                        entry.cached_latest_version = info.latest.clone();
                                        entry.cached_version_check_at = format!("{:?}", std::time::SystemTime::now());
                                    }

                                    if info.has_upgrade {
                                        tracing::info!("Update available for '{}': {} → {}", inst.name, info.current, info.latest);
                                        // No popup — user checks updates from ClawPage manually
                                        let title = if info.is_security_release { "Security Update Available" } else { "Update Available" };
                                        tray::send_notification(&update_handle, title,
                                            &format!("OpenClaw {} → {} for '{}'", info.current, info.latest, inst.name));
                                    }
                                }
                                Err(e) => tracing::debug!("Update check failed for '{}': {e}", inst.name),
                            }
                        }
                        config.save().ok();
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await; // 1h
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
            ipc::exit_app,
        ])
        .on_window_event(|window, event| {
            // Close button hides the MAIN window instead of quitting — so the
            // app can keep running from the system tray. Secondary windows
            // (install wizard, etc.) get normal close behaviour; silently
            // hiding them turned subsequent "Add instance" clicks into no-ops
            // because open_install_window's `get_webview_window(label)` kept
            // returning the hidden window.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let _ = window.hide();
                    api.prevent_close();
                }
                // Other windows: let the default destroy path run, so the next
                // Add click builds a fresh window instead of focusing a hidden
                // ghost with the same label.
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building ClawEnv")
        .run(|app, event| {
            // macOS-only handling: the Reopen event only fires on Apple
            // platforms. On Windows/Linux the closure args are otherwise
            // unused — `let _ = (...)` consumes them so clippy's
            // `unused_variables` stays quiet without an ecosystem-level
            // `#[allow]` sprinkled on the closure signature.
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
