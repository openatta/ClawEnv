//! v2-native SandboxBackend trait + implementations.
//!
//! Replaces our former dep on `clawenv_core::sandbox::*`. Scope is tighter
//! than v1: we model only what `SandboxOps` uses (is_available / start /
//! stop / exec / stats / edit_port_forwards). VM creation/destruction and
//! rootfs import are out of scope — those are one-time bootstrap
//! operations and v1's installer already handles them. v2 manages the
//! *running* VM.

pub mod lima;
pub mod wsl;
pub mod podman;

use async_trait::async_trait;

pub use lima::LimaBackend;
pub use podman::PodmanBackend;
pub use wsl::WslBackend;

/// Capabilities and runtime state of a sandbox backend. Mirrors v2's
/// `SandboxOps` needs.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Human-readable name (e.g. "Lima", "WSL2", "Podman").
    fn name(&self) -> &str;

    /// Instance identifier (e.g. "default").
    fn instance(&self) -> &str;

    /// Is the underlying tool + VM available / running right now?
    async fn is_available(&self) -> anyhow::Result<bool>;

    /// Start the VM (idempotent — noop if already running).
    async fn start(&self) -> anyhow::Result<()>;

    /// Stop the VM (idempotent — noop if already stopped).
    async fn stop(&self) -> anyhow::Result<()>;

    /// Run a shell command inside the VM and return stdout on success.
    /// Error includes stderr snippet on non-zero exit.
    async fn exec(&self, cmd: &str) -> anyhow::Result<String>;

    /// Current resource usage.
    async fn stats(&self) -> anyhow::Result<ResourceStats>;

    /// Replace the full port-forward table with this set.
    /// Individual add/remove is modeled by reading current state + rewriting
    /// at the `SandboxOps` layer.
    async fn edit_port_forwards(&self, forwards: &[(u16, u16)]) -> anyhow::Result<()>;

    fn supports_rename(&self) -> bool { false }
    fn supports_resource_edit(&self) -> bool { false }
    fn supports_port_edit(&self) -> bool { false }
}

#[derive(Debug, Clone, Default)]
pub struct ResourceStats {
    pub cpu_percent: f32,
    pub memory_used_mb: u64,
    pub memory_limit_mb: u64,
}
