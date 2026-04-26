//! Build a fully-populated `ToolRegistry` for the current platform.
//!
//! Mac + win get the real enigo/xcap-backed handlers. Linux gets an
//! empty registry — `tools/list` returns `[]`, `tools/call` returns
//! `invalid_argument`. This keeps the registry buildable on Linux for
//! CLI-only developer flows, matching CLAUDE.md "Linux GUI 不支持".

use std::sync::Arc;

use super::registry::{ToolHandler, ToolRegistry};

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn build_default(max_screenshot_dim: u32) -> ToolRegistry {
    use super::{keyboard, mouse, screen};

    let handlers: Vec<Arc<dyn ToolHandler>> = vec![
        Arc::new(keyboard::TypeText),
        Arc::new(keyboard::PressKey),
        Arc::new(mouse::MouseMove),
        Arc::new(mouse::MouseClick),
        Arc::new(mouse::MouseScroll),
        Arc::new(mouse::MouseDrag),
        Arc::new(screen::ScreenInfo),
        Arc::new(screen::ScreenCapture { max_dim: max_screenshot_dim }),
    ];
    ToolRegistry::new(handlers)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn build_default(_max_screenshot_dim: u32) -> ToolRegistry {
    tracing::warn!(
        target: "clawenv::input",
        "input/screen MCP tools not available on this platform; registry is empty"
    );
    ToolRegistry::new(Vec::<Arc<dyn ToolHandler>>::new())
}
