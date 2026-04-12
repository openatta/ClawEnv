use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;

/// Tray icon status
#[derive(Clone, Copy, PartialEq)]
pub enum TrayStatus {
    Running,
    Stopped,
    Error,
}

/// Generate a simple 16x16 RGBA solid-color circle icon
fn make_circle_icon(r: u8, g: u8, b: u8) -> tauri::image::Image<'static> {
    let (w, h) = (16u32, 16u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let radius = cx - 1.5;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= radius * radius {
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    tauri::image::Image::new_owned(rgba, w, h)
}

/// Update the tray icon based on status
pub fn set_tray_status(app: &AppHandle, status: TrayStatus) {
    let icon = match status {
        TrayStatus::Running => make_circle_icon(34, 197, 94),   // green
        TrayStatus::Stopped => make_circle_icon(156, 163, 175), // gray
        TrayStatus::Error => make_circle_icon(239, 68, 68),     // red
    };

    if let Some(tray) = app.tray_by_id("clawenv-tray") {
        let _ = tray.set_icon(Some(icon));
    }

    let tooltip = match status {
        TrayStatus::Running => "ClawEnv — Running",
        TrayStatus::Stopped => "ClawEnv — Stopped",
        TrayStatus::Error => "ClawEnv — Error",
    };
    update_tray_tooltip(app, tooltip);
}

/// Send a system notification via tauri-plugin-notification
pub fn send_notification(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

/// Update the tray tooltip to reflect current state
pub fn update_tray_tooltip(app: &AppHandle, status_text: &str) {
    if let Some(tray) = app.tray_by_id("clawenv-tray") {
        let _ = tray.set_tooltip(Some(status_text));
    }
}

/// Build the right-click context menu with instance sub-menus and actions.
/// Instance labels show status: 🟢 Running, 🔴 Stopped, ⚪ Unknown.
fn build_tray_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    let menu = Menu::new(app)?;

    if let Ok(config) = ConfigManager::load() {
        for inst in config.instances() {
            // Quick health check via backend (sync-safe: spawn + block briefly)
            let status_icon = get_instance_status_icon(inst);

            let status_text = if status_icon == "ON" { "[ON]" } else { "[OFF]" };
            let label = format!("{} {} - {}", status_text, inst.name, inst.claw_type);
            let submenu = Submenu::with_id(
                app,
                &format!("submenu-{}", inst.name),
                &label,
                true,
            )?;

            let is_running = status_icon == "ON";

            if is_running {
                let stop_item = MenuItem::with_id(app, &format!("stop-{}", inst.name), "Stop", true, None::<&str>)?;
                let restart_item = MenuItem::with_id(app, &format!("restart-{}", inst.name), "Restart", true, None::<&str>)?;
                submenu.append(&stop_item)?;
                submenu.append(&restart_item)?;
            } else {
                let start_item = MenuItem::with_id(app, &format!("start-{}", inst.name), "Start", true, None::<&str>)?;
                submenu.append(&start_item)?;
            }

            let sep = PredefinedMenuItem::separator(app)?;
            let logs_item = MenuItem::with_id(app, &format!("logs-{}", inst.name), "View Logs", true, None::<&str>)?;
            submenu.append(&sep)?;
            submenu.append(&logs_item)?;

            menu.append(&submenu)?;
        }

        if !config.instances().is_empty() {
            let sep = PredefinedMenuItem::separator(app)?;
            menu.append(&sep)?;
        }
    }

    let open_item = MenuItem::with_id(app, "open", "Open ClawEnv", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit ClawEnv", true, None::<&str>)?;

    menu.append(&open_item)?;
    menu.append(&sep2)?;
    menu.append(&quit_item)?;

    Ok(menu)
}

/// Get a status icon for an instance (called from sync tray menu build).
/// Uses a quick TCP connect to the gateway port to check if it's running.
fn get_instance_status_icon(inst: &clawenv_core::config::InstanceConfig) -> &'static str {
    use std::net::TcpStream;
    use std::time::Duration;

    let addr = format!("127.0.0.1:{}", inst.gateway.gateway_port);
    match TcpStream::connect_timeout(
        &addr.parse().unwrap_or_else(|_| "127.0.0.1:3000".parse().unwrap()),
        Duration::from_secs(1),
    ) {
        Ok(_) => "ON",
        Err(_) => "OFF",
    }
}

pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_tray_menu(app)?;

    let _tray = TrayIconBuilder::with_id("clawenv-tray")
        .tooltip("ClawEnv")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                "open" => {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
                "settings" => {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                        let _ = tauri::Emitter::emit(app, "navigate", "settings");
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                other => {
                    // Handle instance actions: start-{name}, stop-{name}, restart-{name}, logs-{name}
                    if let Some(name) = other.strip_prefix("start-") {
                        let app_handle = app.clone();
                        let instance_name = name.to_string();
                        tauri::async_runtime::spawn(async move {
                            match ConfigManager::load() {
                                Ok(config) => {
                                    match instance::get_instance(&config, &instance_name) {
                                        Ok(inst) => {
                                            if let Err(e) = instance::start_instance(inst).await {
                                                tracing::error!("Failed to start {}: {}", instance_name, e);
                                                send_notification(&app_handle, "Start Failed", &format!("Instance '{}': {}", instance_name, e));
                                            } else {
                                                send_notification(&app_handle, "Instance Started", &format!("'{}' is now running", instance_name));
                                                let _ = refresh_tray(&app_handle);
                                            }
                                        }
                                        Err(e) => tracing::error!("Instance not found: {}", e),
                                    }
                                }
                                Err(e) => tracing::error!("Failed to load config: {}", e),
                            }
                        });
                    } else if let Some(name) = other.strip_prefix("stop-") {
                        let app_handle = app.clone();
                        let instance_name = name.to_string();
                        tauri::async_runtime::spawn(async move {
                            match ConfigManager::load() {
                                Ok(config) => {
                                    match instance::get_instance(&config, &instance_name) {
                                        Ok(inst) => {
                                            if let Err(e) = instance::stop_instance(inst).await {
                                                tracing::error!("Failed to stop {}: {}", instance_name, e);
                                                send_notification(&app_handle, "Stop Failed", &format!("Instance '{}': {}", instance_name, e));
                                            } else {
                                                send_notification(&app_handle, "Instance Stopped", &format!("'{}' has been stopped", instance_name));
                                                let _ = refresh_tray(&app_handle);
                                            }
                                        }
                                        Err(e) => tracing::error!("Instance not found: {}", e),
                                    }
                                }
                                Err(e) => tracing::error!("Failed to load config: {}", e),
                            }
                        });
                    } else if let Some(name) = other.strip_prefix("restart-") {
                        let app_handle = app.clone();
                        let instance_name = name.to_string();
                        tauri::async_runtime::spawn(async move {
                            match ConfigManager::load() {
                                Ok(config) => {
                                    match instance::get_instance(&config, &instance_name) {
                                        Ok(inst) => {
                                            if let Err(e) = instance::restart_instance(inst).await {
                                                tracing::error!("Failed to restart {}: {}", instance_name, e);
                                                send_notification(&app_handle, "Restart Failed", &format!("Instance '{}': {}", instance_name, e));
                                            } else {
                                                send_notification(&app_handle, "Instance Restarted", &format!("'{}' has been restarted", instance_name));
                                                let _ = refresh_tray(&app_handle);
                                            }
                                        }
                                        Err(e) => tracing::error!("Instance not found: {}", e),
                                    }
                                }
                                Err(e) => tracing::error!("Failed to load config: {}", e),
                            }
                        });
                    } else if let Some(name) = other.strip_prefix("logs-") {
                        // Open main window and navigate to logs for this instance
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                            let _ = tauri::Emitter::emit(app, "navigate", format!("logs/{}", name));
                        }
                    }
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Double-click: show/focus main window
            if let TrayIconEvent::DoubleClick { .. } = event {
                if let Some(win) = tray.app_handle().get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Rebuild the tray menu based on current instance states.
/// Call this after instance status changes.
pub fn refresh_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_tray_menu(app)?;
    if let Some(tray) = app.tray_by_id("clawenv-tray") {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}
