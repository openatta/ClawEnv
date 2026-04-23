//! SandboxOps — high-level lifecycle + port + doctor operations for VM backends.

pub mod types;
pub mod ops;
pub mod lima;
pub mod wsl;
pub mod podman;

pub use ops::SandboxOps;
pub use types::{
    BackendKind, DoctorIssue, PortRule, ResourceStats, SandboxCaps, SandboxDoctorReport,
    SandboxStatus, Severity, VmState,
};
pub use lima::LimaOps;
pub use wsl::WslOps;
pub use podman::PodmanOps;
