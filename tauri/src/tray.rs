use tauri::{
    tray::{TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

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
    if let Some(tray) = app.tray_by_id("clawenv-tray") {
        let _ = tray.set_tooltip(Some(tooltip));
    }
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

/// Set up system tray.
///
/// Left-click: show main window.
/// Right-click: show a small popup WebView window near the tray as a menu
/// (workaround for Windows ARM64 where native tray menus render as 0-width).
pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let _tray = TrayIconBuilder::with_id("clawenv-tray")
        .tooltip("ClawEnv")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            match event {
                TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    position,
                    ..
                } => {
                    let app = tray.app_handle();
                    // If tray popup exists, close it
                    if let Some(popup) = app.get_webview_window("tray-popup") {
                        let _ = popup.close();
                    }
                    // Show main window
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.unminimize();
                        let _ = win.set_focus();
                    }
                }
                TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Right,
                    position,
                    ..
                } => {
                    let app = tray.app_handle();
                    // Toggle tray popup window
                    if let Some(popup) = app.get_webview_window("tray-popup") {
                        let _ = popup.close();
                        return;
                    }

                    // Create a small popup window near the tray icon
                    let x = position.x as f64 - 100.0;
                    let y = position.y as f64 - 200.0;

                    let url = "/index.html?mode=tray-popup";
                    if let Ok(popup) = tauri::webview::WebviewWindowBuilder::new(
                        app,
                        "tray-popup",
                        tauri::WebviewUrl::App(url.into()),
                    )
                    .title("")
                    .inner_size(200.0, 180.0)
                    .position(x.max(0.0), y.max(0.0))
                    .resizable(false)
                    .decorations(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .build()
                    {
                        // Close popup when it loses focus
                        let popup_handle = popup.clone();
                        popup.on_window_event(move |event| {
                            if let tauri::WindowEvent::Focused(false) = event {
                                let _ = popup_handle.close();
                            }
                        });
                    }
                }
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

/// Refresh tray state (icon color based on instance health).
pub fn refresh_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use clawenv_core::config::ConfigManager;
    use std::net::TcpStream;
    use std::time::Duration;

    if let Ok(config) = ConfigManager::load() {
        let any_running = config.instances().iter().any(|inst| {
            let addr = format!("127.0.0.1:{}", inst.gateway.gateway_port);
            TcpStream::connect_timeout(
                &addr.parse().unwrap_or_else(|_| "127.0.0.1:3000".parse().unwrap()),
                Duration::from_secs(1),
            ).is_ok()
        });

        if config.instances().is_empty() {
            set_tray_status(app, TrayStatus::Stopped);
        } else if any_running {
            set_tray_status(app, TrayStatus::Running);
        } else {
            set_tray_status(app, TrayStatus::Stopped);
        }
    }

    Ok(())
}
