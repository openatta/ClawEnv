//! Build a fully-populated `ToolRegistry` for the current platform.
//! Mac + win get the real enigo/xcap-backed handlers (each wrapped in
//! the kill-switch gate). Linux gets an empty registry with no tools —
//! `tools/list` returns `[]`, `tools/call` returns `invalid_argument
//! "unknown tool"`. This keeps the MCP server itself buildable on Linux
//! for CLI-only developer flows.

use std::sync::Arc;

use super::registry::{ToolHandler, ToolRegistry};
use crate::remote::killswitch::{GatedToolHandler, KillSwitchState};

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn build_default(max_screenshot_dim: u32, kill: KillSwitchState) -> ToolRegistry {
    use super::{keyboard, mouse, screen};

    // Input-affecting tools go through the gate; pure-read tools
    // (screen_info, screen_capture) do NOT — a user hitting the
    // kill-switch wants to stop the remote from typing/clicking, not
    // from reading state they already permitted.
    let gate = |h: Arc<dyn ToolHandler>| -> Arc<dyn ToolHandler> {
        Arc::new(GatedToolHandler::new(h, kill.clone()))
    };

    let handlers: Vec<Arc<dyn ToolHandler>> = vec![
        gate(Arc::new(keyboard::TypeText)),
        gate(Arc::new(keyboard::PressKey)),
        gate(Arc::new(mouse::MouseMove)),
        gate(Arc::new(mouse::MouseClick)),
        gate(Arc::new(mouse::MouseScroll)),
        gate(Arc::new(mouse::MouseDrag)),
        Arc::new(screen::ScreenInfo),
        Arc::new(screen::ScreenCapture { max_dim: max_screenshot_dim }),
    ];
    ToolRegistry::new(handlers)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn build_default(_max_screenshot_dim: u32, _kill: KillSwitchState) -> ToolRegistry {
    tracing::warn!(
        target: "clawenv::input",
        "input/screen MCP tools not available on this platform; registry is empty"
    );
    ToolRegistry::new(Vec::<Arc<dyn ToolHandler>>::new())
}
