//! GUI-side bridges between v2 `clawops_core` and Tauri IPC handlers.
//!
//! v2 `InstanceOrchestrator` deliberately doesn't expose
//! "give me the SandboxBackend for this instance" or "stop this instance"
//! as standalone helpers — those workflows go through orchestrator methods
//! that own progress reporting + registry mutation. The GUI sometimes
//! needs the raw backend (probing browser status, executing diagnostic
//! commands) outside that flow, so the small adapter lives here.

use std::sync::Arc;

use clawops_core::instance::{InstanceConfig, SandboxKind};
use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};

/// Construct a fresh `Box<dyn SandboxBackend>` for this instance, mirroring
/// what `InstanceOrchestrator` does internally. Errors when the instance
/// is `Native` (no sandbox backend exists) — callers that may receive a
/// native instance should branch on `inst.backend == SandboxKind::Native`
/// before calling.
pub fn backend_for_instance(inst: &InstanceConfig) -> Result<Box<dyn SandboxBackend>, String> {
    let id: String = if inst.sandbox_instance.is_empty() {
        inst.name.clone()
    } else {
        inst.sandbox_instance.clone()
    };
    match inst.backend {
        SandboxKind::Lima => Ok(Box::new(LimaBackend::new(id))),
        SandboxKind::Wsl2 => Ok(Box::new(WslBackend::new(id))),
        SandboxKind::Podman => Ok(Box::new(PodmanBackend::new(id))),
        SandboxKind::Native => Err(format!(
            "instance `{}` is native — no sandbox backend",
            inst.name
        )),
    }
}

/// `Arc` flavour of [`backend_for_instance`] for callers like
/// `ChromiumBackend::new` that take an `Arc<dyn SandboxBackend>`.
pub fn backend_arc_for_instance(inst: &InstanceConfig) -> Result<Arc<dyn SandboxBackend>, String> {
    backend_for_instance(inst).map(Arc::from)
}

/// Look up the host-side gateway port. v2 stores all forwards in
/// `inst.ports` keyed by label; this returns the binding labelled
/// `gateway` (or 0 when absent — which signals "not yet configured" to
/// the GUI's button-state logic).
pub fn gateway_port(inst: &InstanceConfig) -> u16 {
    inst.ports.iter().find(|p| p.label == "gateway").map(|p| p.host).unwrap_or(0)
}

/// Same as [`gateway_port`] for the optional ttyd terminal forward.
pub fn ttyd_port(inst: &InstanceConfig) -> u16 {
    inst.ports.iter().find(|p| p.label == "ttyd").map(|p| p.host).unwrap_or(0)
}
