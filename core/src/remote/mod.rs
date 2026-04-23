//! Remote desktop control runtime. See `docs/26-remote-desktop-control.md`.
//!
//! Layering:
//! - `protocol`    — JSON frame types for the WSS reverse channel
//! - `wss_client`  — tokio-tungstenite connection (low level, one-shot)
//! - `supervisor`  — reconnect + heartbeat state machine wrapping wss_client
//! - `dispatcher`  — routes ServerMsg into the claw agent (stub in Phase A)
//! - `audit`       — append-only JSONL log for every remote-initiated action

pub mod agent;
pub mod audit;
pub mod dispatcher;
pub mod killswitch;
pub mod protocol;
pub mod runtime;
pub mod supervisor;
pub mod wss_client;

pub use agent::{AgentInvoker, EchoInvoker, HttpGatewayInvoker};
pub use protocol::{BridgeMsg, ServerMsg};
pub use runtime::{start as start_runtime, RuntimeHandle, RuntimeOptions};
pub use supervisor::{Status, SupervisorConfig, SupervisorHandle};
