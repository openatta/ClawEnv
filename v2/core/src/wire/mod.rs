//! Wire types — clawcli `--json` Data event payloads.
//!
//! Per `v2/docs/CLI-DESIGN.md` §5: each verb that emits a Data event
//! conforms to a documented type defined here. These types are the
//! contract between clawcli and any consumer (GUI, scripts, CI tools).
//!
//! Naming is v2-clean — no v1 carry-overs. Frontend TypeScript types
//! are the consumers and adapt to this shape.

use serde::{Deserialize, Serialize};

// ——— Instance list / status ———

/// Common fields for every "tell me about an instance" Data payload.
/// `list` returns `Vec<InstanceSummary>`, `status` returns it inline
/// + extra capability info.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstanceSummary {
    /// User-chosen instance name (unique key in the registry).
    pub name: String,
    /// Claw product id ("openclaw", "hermes", …).
    pub claw: String,
    /// Backend kind: `native` | `lima` | `wsl2` | `podman`. Same
    /// strings v2 `SandboxKind::as_str()` returns — no "-alpine" suffix.
    pub backend: String,
    /// VM/container identifier (Lima instance name, WSL distro,
    /// Podman container). Empty for native.
    pub sandbox_instance: String,
    /// Claw version string as reported by `<bin> --version`. Empty
    /// when not yet probed (list emits empty; status fills it on
    /// probe).
    #[serde(default)]
    pub version: String,
    /// Health: `ok` | `stopped` | `broken` | `missing` | `unknown`.
    pub health: String,
    /// Gateway HTTP port on the host. 0 when no gateway port was assigned.
    pub gateway_port: u16,
    /// Terminal-over-WebSocket port. 0 when not configured.
    #[serde(default)]
    pub ttyd_port: u16,
    /// Dashboard port for claws with split UI (Hermes). 0 means the
    /// claw uses gateway_port for its UI (OpenClaw).
    #[serde(default)]
    pub dashboard_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListResponse {
    pub instances: Vec<InstanceSummary>,
}

/// Backend capabilities for one instance. Returned inline in
/// `StatusResponse.capabilities`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilitiesInfo {
    pub rename: bool,
    pub resource_edit: bool,
    pub port_edit: bool,
    /// Whether the backend supports VM snapshots (Lima yes, others no).
    #[serde(default)]
    pub snapshot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusResponse {
    /// Flatten the summary fields inline so consumers can `s.name` /
    /// `s.health` instead of `s.summary.name`.
    #[serde(flatten)]
    pub summary: InstanceSummary,
    /// Backend capabilities. None for native instances (no VM caps).
    #[serde(default)]
    pub capabilities: Option<CapabilitiesInfo>,
}

// ——— Logs / Exec ———

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogResponse {
    /// Concatenated log content (newline-delimited).
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecResult {
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    pub exit_code: i32,
}

// ——— Doctor ———

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorIssueWire {
    pub id: String,
    /// `info` | `warn` | `error`
    pub severity: String,
    pub message: String,
    #[serde(default)]
    pub repair_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorReport {
    /// Instance name when scoped to one; "host" for the composite
    /// host doctor (no `<name>` arg).
    pub scope: String,
    pub healthy: bool,
    pub issues: Vec<DoctorIssueWire>,
    pub checked_at: String,
}

// ——— Net check ———

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetCheckHostResult {
    pub host: String,
    pub reachable: bool,
    #[serde(default)]
    pub http_status: Option<u16>,
    #[serde(default)]
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetCheckReport {
    /// Where the probe ran from: `host` or `sandbox`.
    pub origin: String,
    pub all_reachable: bool,
    pub hosts: Vec<NetCheckHostResult>,
    #[serde(default)]
    pub suggestion: Option<String>,
}

// ——— Export / Import ———

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportReport {
    pub instance: String,
    pub claw: String,
    pub backend: String,
    pub output: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportReport {
    pub instance: InstanceSummary,
    /// Bundle metadata for traceability — what the bundle says about
    /// itself, regardless of where it ended up.
    pub source_clawenv_version: String,
    pub source_platform: String,
    pub source_created_at: String,
}

// ——— Update check ———

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateCheckResponse {
    pub current: String,
    pub latest: String,
    pub has_upgrade: bool,
    #[serde(default)]
    pub is_security_release: bool,
    #[serde(default)]
    pub changelog: String,
}

// ——— System info ———

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub memory_gb: f64,
    pub disk_free_gb: f64,
    /// Default sandbox backend for this host (`lima`/`wsl2`/`podman`).
    pub sandbox_backend: String,
    pub sandbox_available: bool,
    /// One row per check the wizard / doctor wants to display. Detail
    /// is human text — `info_only` flags advisory rows the GUI greys
    /// out instead of showing as red.
    pub checks: Vec<SystemCheckItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemCheckItem {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    #[serde(default)]
    pub info_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionInfo {
    pub clawcli_version: String,
    pub commit: String,
    pub build_date: String,
    pub capabilities: Vec<String>,
}

// ——— Claw types ———

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClawTypeInfo {
    pub id: String,
    pub display_name: String,
    /// `npm` | `pip` | `git_pip`.
    pub package_manager: String,
    /// Package identifier within the manager (e.g. npm package name,
    /// pip name, or `git+https://...`).
    pub package_id: String,
    pub default_port: u16,
    pub supports_mcp: bool,
    pub supports_browser: bool,
    pub supports_native: bool,
    /// Whether the claw uses its gateway as the UI (OpenClaw) vs.
    /// having a separate dashboard process (Hermes).
    pub has_gateway_ui: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClawTypesResponse {
    pub claw_types: Vec<ClawTypeInfo>,
}

// ——— Sandbox layer ———

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxVmInfo {
    pub name: String,
    pub status: String,
    /// Whether this VM is in v2's instance registry.
    pub managed: bool,
    /// User-chosen instance name when `managed=true`. Empty otherwise.
    #[serde(default)]
    pub instance_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxListResponse {
    /// Backend kind that owned the listed VMs (`lima`/`wsl2`/`podman`).
    pub backend: String,
    pub vms: Vec<SandboxVmInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SandboxStats {
    pub backend: String,
    pub instance: String,
    pub cpu_percent: f32,
    pub memory_used_mb: u64,
    pub memory_limit_mb: u64,
    pub disk_used_gb: f64,
    pub disk_total_gb: f64,
}

// ——— Helpers: v2 → wire conversion ———

impl InstanceSummary {
    /// Build from `instance::InstanceConfig` plus a probed VM state.
    /// Fields v2 doesn't track yet (version) get blank; callers who
    /// need them call `clawcli status <name>` (which probes).
    pub fn from_instance(
        cfg: &crate::instance::InstanceConfig,
        vm_state: Option<&str>,
    ) -> Self {
        let gateway_port = cfg.ports.iter()
            .find(|p| p.label == "gateway")
            .map(|p| p.host).unwrap_or(0);
        let ttyd_port = cfg.ports.iter()
            .find(|p| p.label == "ttyd")
            .map(|p| p.host).unwrap_or(0);
        let dashboard_port = cfg.ports.iter()
            .find(|p| p.label == "dashboard")
            .map(|p| p.host).unwrap_or(0);
        let health = match vm_state {
            Some("running") => "ok",
            Some("stopped") => "stopped",
            Some("broken") => "broken",
            Some("missing") => "missing",
            _ => "unknown",
        }.to_string();
        let sandbox_instance = if cfg.sandbox_instance.is_empty() {
            // Native: no VM, but use the instance name for identity.
            // List consumers can branch on backend == "native".
            cfg.name.clone()
        } else {
            cfg.sandbox_instance.clone()
        };
        Self {
            name: cfg.name.clone(),
            claw: cfg.claw.clone(),
            backend: cfg.backend.as_str().into(),
            sandbox_instance,
            version: String::new(),
            health,
            gateway_port,
            ttyd_port,
            dashboard_port,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{InstanceConfig, PortBinding, SandboxKind};

    fn cfg(backend: SandboxKind) -> InstanceConfig {
        InstanceConfig {
            name: "test".into(),
            claw: "openclaw".into(),
            backend,
            sandbox_instance: "test-vm".into(),
            ports: vec![
                PortBinding { host: 3001, guest: 3000, label: "gateway".into() },
                PortBinding { host: 3002, guest: 3001, label: "ttyd".into() },
            ],
            created_at: "2026-04-25T00:00:00Z".into(),
            updated_at: String::new(),
            note: String::new(),
        }
    }

    #[test]
    fn from_instance_uses_v2_backend_strings() {
        let s = InstanceSummary::from_instance(&cfg(SandboxKind::Lima), Some("running"));
        assert_eq!(s.gateway_port, 3001);
        assert_eq!(s.ttyd_port, 3002);
        assert_eq!(s.dashboard_port, 0);
        assert_eq!(s.health, "ok");
        assert_eq!(s.backend, "lima"); // not "lima-alpine" as v1 had
        assert_eq!(s.sandbox_instance, "test-vm");
    }

    #[test]
    fn native_backend_string_is_native() {
        let mut c = cfg(SandboxKind::Native);
        c.sandbox_instance = String::new();
        let s = InstanceSummary::from_instance(&c, None);
        assert_eq!(s.backend, "native");
        // Native uses instance name as sandbox_instance for identity.
        assert_eq!(s.sandbox_instance, "test");
        assert_eq!(s.health, "unknown");
    }

    #[test]
    fn status_response_flattens_summary() {
        // The frontend reads s.name / s.health directly; serde flatten
        // makes that work without a nested .summary level.
        let r = StatusResponse {
            summary: InstanceSummary::from_instance(&cfg(SandboxKind::Lima), Some("running")),
            capabilities: Some(CapabilitiesInfo {
                rename: true, resource_edit: true, port_edit: true, snapshot: false,
            }),
        };
        let v = serde_json::to_value(&r).unwrap();
        // Top-level fields, not nested under "summary".
        assert_eq!(v["name"], "test");
        assert_eq!(v["claw"], "openclaw");
        assert_eq!(v["health"], "ok");
        assert!(v["capabilities"].is_object());
    }
}
