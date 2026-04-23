//! Native-host input (keyboard, mouse, screen-capture) backing the MCP
//! tools exposed by the bridge. Tools are platform-gated — mac + win only
//! per CLAUDE.md. Linux builds get `ToolHandler` implementations that
//! return `unsupported`, so the MCP registry compiles everywhere without
//! pulling in `enigo` / `xcap`.
//!
//! Exposed as a library module so the same registry can be reused by the
//! Tauri app when it wants to host the MCP server in-process.

pub mod registry;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod keyboard;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod mouse;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod screen;
pub mod factory;
pub mod perm;

pub use factory::build_default;
pub use registry::{ToolError, ToolHandler, ToolRegistry, ToolSpec};
