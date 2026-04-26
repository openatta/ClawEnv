//! Embedded HIL + exec-approval + hardware-device HTTP server.
//!
//! Lifted verbatim from v1 `clawenv_core::bridge` because v2 `clawops-core`
//! intentionally stays headless. The full AttaRun bridge daemon is the
//! long-term home for these endpoints (tracked in
//! `docs/v2/v0.5.x-features.md` "Bridge admin UI"); until then the GUI keeps
//! the server in-process so HIL pop-ups and exec approvals keep working.

pub mod mcp;
pub mod permissions;
pub mod server;

pub use mcp::McpState;
pub use permissions::BridgePermissions;
pub use server::{start_bridge, EventEmitter};
