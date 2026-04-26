//! Shared response types for CLI JSON output.
//!
//! CLI serializes these with `serde_json::to_value()`.
//! Tauri deserializes with `serde_json::from_value()`.
//! No manual `.get("field").and_then()` needed.

use serde::{Serialize, Deserialize};

// ---- Instance ----
//
// Field names mirror v2 wire shapes (clawcli `--json` output) — not v1's
// historical `claw_type`/`sandbox_type`/`sandbox_id` triple. The Tauri
// IPC layer maps these into TypeScript-facing structs (`InstanceInfo`)
// where the GUI's existing field names are preserved.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceSummary {
    pub name: String,
    /// Claw product id (`openclaw`/`hermes`).
    pub claw: String,
    /// Backend kind: `native`/`lima`/`wsl2`/`podman`.
    pub backend: String,
    /// VM/container identifier (Lima instance, WSL distro, Podman
    /// container). Empty for native; same as `name` when no separate
    /// sandbox identity exists.
    #[serde(default)]
    pub sandbox_instance: String,
    #[serde(default)]
    pub version: String,
    pub health: String,
    pub gateway_port: u16,
    #[serde(default)]
    pub ttyd_port: u16,
    /// Web dashboard port. `0` when the claw uses its gateway process as
    /// the UI (OpenClaw); non-zero for claws with a split UI process
    /// (Hermes). Frontend opens `http://127.0.0.1:{dashboard_port || gateway_port}/`.
    #[serde(default)]
    pub dashboard_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse {
    pub instances: Vec<InstanceSummary>,
}

/// Wraps multi-line log output into a single Data event payload.
/// v2 emits an object (`{"content": "..."}`) instead of a bare JSON
/// string so future fields (e.g. `truncated_at`, `file_paths`) can be
/// added without breaking consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogResponse {
    pub content: String,
}

/// `clawcli status` flattens the InstanceSummary fields inline (serde
/// flatten on the v2 side). Using `#[serde(flatten)]` here mirrors that
/// — `s.name` / `s.health` / etc. are read from top-level keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    #[serde(flatten)]
    pub summary: InstanceSummary,
    #[serde(default)]
    pub capabilities: Option<CapabilitiesInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitiesInfo {
    pub rename: bool,
    pub resource_edit: bool,
    pub port_edit: bool,
    #[serde(default)]
    pub snapshot: bool,
}

// ---- System Check ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemCheckResponse {
    pub os: String,
    pub arch: String,
    pub memory_gb: f64,
    pub disk_free_gb: f64,
    pub sandbox_backend: String,
    pub sandbox_available: bool,
    pub checks: Vec<CheckItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    /// If true, this check is informational (installer will handle it).
    /// Frontend shows as gray instead of red.
    #[serde(default)]
    pub info_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorResponse {
    pub os: String,
    pub arch: String,
    pub memory_gb: String,
    pub disk_free_gb: String,
    pub sandbox_backend: String,
    pub sandbox_available: bool,
    pub instances: usize,
}

// ---- Claw Types ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawTypeInfo {
    pub id: String,
    pub display_name: String,
    pub package_manager: String,
    /// Package identifier within the manager (npm name, pip name, or
    /// `git+https://...` URL). Replaces v1's split `npm_package` /
    /// `pip_package` pair.
    pub package_id: String,
    pub default_port: u16,
    pub supports_mcp: bool,
    pub supports_browser: bool,
    pub has_gateway_ui: bool,
    pub supports_native: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawTypesResponse {
    pub claw_types: Vec<ClawTypeInfo>,
}

// ---- Update ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckResponse {
    pub current: String,
    pub latest: String,
    pub has_upgrade: bool,
    pub is_security_release: bool,
    pub changelog: String,
}

// ---- Sandbox ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxVmInfo {
    pub name: String,
    pub status: String,
    /// Whether this VM is managed by ClawEnv
    pub managed: bool,
    /// User-chosen instance name when `managed=true`. Empty otherwise —
    /// matches v2 wire (no Option wrapper; empty string = none).
    #[serde(default)]
    pub instance_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxListResponse {
    /// Backend kind that owned the listed VMs (`lima`/`wsl2`/`podman`).
    pub backend: String,
    pub vms: Vec<SandboxVmInfo>,
}

// ---- Config ----

/// Config show response — field names match `config set` key names (dot notation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigShowResponse {
    pub language: String,
    pub theme: String,
    pub user_mode: String,
    #[serde(rename = "proxy.enabled")]
    pub proxy_enabled: bool,
    #[serde(rename = "proxy.http")]
    pub proxy_http: String,
    #[serde(rename = "proxy.https")]
    pub proxy_https: String,
    #[serde(rename = "proxy.no_proxy")]
    pub proxy_no_proxy: String,
    #[serde(rename = "bridge.enabled")]
    pub bridge_enabled: bool,
    #[serde(rename = "bridge.port")]
    pub bridge_port: u16,
    #[serde(rename = "updates.auto_check")]
    pub updates_auto_check: bool,
    pub instances_count: usize,
}
