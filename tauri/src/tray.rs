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

/// Update the tray tooltip based on status (icon always stays as the Logo)
pub fn set_tray_status(app: &AppHandle, status: TrayStatus) {
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

/// Build a simple tray right-click menu (pure ASCII, no emoji).
fn build_tray_menu(app: &AppHandle) -> Result<tauri::menu::Menu<tauri::Wry>, Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let menu = Menu::new(app)?;

    let open_item = MenuItem::with_id(app, "open", "Open ClawEnv", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    menu.append(&open_item)?;
    menu.append(&sep)?;
    menu.append(&quit_item)?;

    Ok(menu)
}

/// Set up system tray.
///
/// Left-click / double-click: show main window.
/// Right-click: native context menu (Open / Quit).
pub fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let menu = build_tray_menu(app)?;

    let _tray = TrayIconBuilder::with_id("clawenv-tray")
        .tooltip("ClawEnv")
        .icon(tauri::include_image!("./icons/32x32.png"))
        .menu(&menu)
        .show_menu_on_left_click(false) // right-click shows menu, left-click handled below
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                "open" => {
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.unminimize();
                        let _ = win.set_focus();
                    }
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Left-click or double-click: show main window
            match event {
                TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left, ..
                }
                | TrayIconEvent::DoubleClick { .. } => {
                    let app = tray.app_handle();
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.unminimize();
                        let _ = win.set_focus();
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
