//! Shared types for SandboxOps.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Lima,
    Wsl2,
    Podman,
}

impl BackendKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lima => "lima",
            Self::Wsl2 => "wsl2",
            Self::Podman => "podman",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VmState {
    Running,
    Stopped,
    /// VM definition exists but won't start / has errors.
    Broken,
    /// VM not created yet on this machine.
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxStatus {
    pub backend: BackendKind,
    pub instance_name: String,
    pub state: VmState,
    pub cpu_cores: Option<u32>,
    pub memory_mb: Option<u32>,
    pub disk_gb: Option<u32>,
    pub ip: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SandboxCaps {
    pub supports_rename: bool,
    pub supports_resource_edit: bool,
    pub supports_port_edit: bool,
    pub supports_snapshot: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceStats {
    pub cpu_percent: f32,
    pub memory_used_mb: u64,
    pub memory_limit_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortRule {
    pub host: u16,
    pub guest: u16,
    /// Backend-native identifier for this rule (Lima yaml index, netsh entry key, etc).
    /// Opaque; used by `remove_port` to locate the same rule.
    pub native_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorIssue {
    /// Stable identifier, e.g. "vm-not-running".
    pub id: String,
    pub severity: Severity,
    pub message: String,
    pub repair_hint: Option<String>,
    pub auto_repairable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxDoctorReport {
    pub backend: BackendKind,
    pub instance_name: String,
    pub issues: Vec<DoctorIssue>,
    pub checked_at: String,
}

impl SandboxDoctorReport {
    pub fn healthy(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Error)
    }
}
