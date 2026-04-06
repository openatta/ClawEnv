// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;
mod ipc;

use clawenv_core::config::ConfigManager;
use clawenv_core::launcher;
use clawenv_core::monitor::InstanceMonitor;
use tauri::Emitter;

fn main() {
    // Initialize logging — visible when run from terminal
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Initialize system tray
            tray::setup_tray(app.handle())?;

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
            tauri::async_runtime::spawn(async move {
                if let Ok(config) = ConfigManager::load() {
                    let bridge_cfg = &config.config().clawenv.bridge;
                    if bridge_cfg.enabled {
                        tracing::info!("Starting bridge server on port {}", bridge_cfg.port);
                        if let Err(e) = clawenv_core::bridge::server::start_bridge(
                            bridge_cfg.port,
                            bridge_cfg.permissions.clone(),
                        ).await {
                            tracing::error!("Bridge server failed: {e}");
                        }
                    }
                }
            });

            // Spawn background instance health monitor
            let monitor_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Wait a moment for config to be ready
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                let config = match ConfigManager::load() {
                    Ok(c) => c,
                    Err(_) => return, // No config yet, nothing to monitor
                };

                let instances = config.instances().to_vec();
                if instances.is_empty() {
                    return;
                }

                let interval = config.config().clawenv.tray.monitor_interval_sec;
                let monitor = InstanceMonitor::with_interval(interval);
                let (tx, mut rx) = tokio::sync::mpsc::channel::<clawenv_core::monitor::HealthEvent>(32);

                // Forward health events to frontend, refresh tray, and send notifications on changes
                let emit_handle = monitor_handle.clone();
                tokio::spawn(async move {
                    use std::collections::HashMap;
                    use clawenv_core::monitor::InstanceHealth;

                    let mut prev_health: HashMap<String, InstanceHealth> = HashMap::new();

                    while let Some(event) = rx.recv().await {
                        let _ = emit_handle.emit("instance-health", &event);
                        // Refresh tray menu to reflect new status
                        let _ = tray::refresh_tray(&emit_handle);

                        // Send notification when health changes
                        if let Some(prev) = prev_health.get(&event.instance_name) {
                            if *prev != event.health {
                                let (title, body) = match event.health {
                                    InstanceHealth::Running => (
                                        "Instance Recovered",
                                        format!("'{}' is now running", event.instance_name),
                                    ),
                                    InstanceHealth::Stopped => (
                                        "Instance Stopped",
                                        format!("'{}' has stopped", event.instance_name),
                                    ),
                                    InstanceHealth::Unreachable => (
                                        "Instance Unreachable",
                                        format!("'{}' is unreachable", event.instance_name),
                                    ),
                                };
                                tray::send_notification(&emit_handle, title, &body);
                                // Update tray icon based on health
                                let tray_status = match event.health {
                                    InstanceHealth::Running => tray::TrayStatus::Running,
                                    InstanceHealth::Stopped => tray::TrayStatus::Stopped,
                                    InstanceHealth::Unreachable => tray::TrayStatus::Error,
                                };
                                tray::set_tray_status(&emit_handle, tray_status);
                            }
                        }
                        prev_health.insert(event.instance_name.clone(), event.health);
                    }
                });

                monitor.run(instances, tx).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::detect_launch_state,
            ipc::get_openclaw_url,
            ipc::list_instances,
            ipc::get_instance_logs,
            ipc::get_instance_status_detail,
            ipc::install_openclaw,
            ipc::start_instance,
            ipc::stop_instance,
            ipc::get_instance_health,
            ipc::save_settings,
            ipc::test_proxy,
            ipc::test_connectivity,
            ipc::detect_system_proxy,
            ipc::system_check,
            ipc::install_prerequisites,
            ipc::test_api_key,
            ipc::install_chromium,
            ipc::get_gateway_token,
            ipc::get_bridge_config,
            ipc::save_bridge_config,
            ipc::start_terminal,
            ipc::write_terminal,
            ipc::close_terminal,
            ipc::open_url_in_browser,
            ipc::create_default_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ClawEnv");
}
