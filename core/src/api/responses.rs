//! Shared response types for CLI JSON output.
//!
//! CLI serializes these with `serde_json::to_value()`.
//! Tauri deserializes with `serde_json::from_value()`.
//! No manual `.get("field").and_then()` needed.

use serde::{Serialize, Deserialize};

// ---- Instance ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceSummary {
    pub name: String,
    pub claw_type: String,
    pub version: String,
    pub sandbox_type: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub name: String,
    pub claw_type: String,
    pub version: String,
    pub sandbox_type: String,
    pub health: String,
    pub gateway_port: u16,
    pub ttyd_port: u16,
    /// See InstanceSummary.dashboard_port — same semantics.
    #[serde(default)]
    pub dashboard_port: u16,
    #[serde(default)]
    pub capabilities: Option<CapabilitiesInfo>,
    #[serde(default)]
    pub gateway_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitiesInfo {
    pub rename: bool,
    pub resource_edit: bool,
    pub port_edit: bool,
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
    pub logo: String,
    pub package_manager: String,
    pub npm_package: String,
    pub pip_package: String,
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
    pub cpus: String,
    pub memory: String,
    pub disk: String,
    pub dir_size: String,
    /// Whether this VM is managed by ClawEnv
    pub managed: bool,
    /// ttyd port for terminal access (only for managed instances)
    #[serde(default)]
    pub ttyd_port: Option<u16>,
    /// User-chosen instance name in config.toml. The VM `name` field holds the
    /// `sandbox_id` (an auto-generated `clawenv-<hash>`), which does NOT equal
    /// the instance name — callers that need to invoke instance-scoped IPCs
    /// (install_chromium, export_sandbox, delete_instance) must use this.
    #[serde(default)]
    pub instance_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxListResponse {
    pub vms: Vec<SandboxVmInfo>,
    pub total_disk_usage: String,
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
    #[serde(rename = "mirrors.preset")]
    pub mirrors_preset: String,
    #[serde(rename = "bridge.enabled")]
    pub bridge_enabled: bool,
    #[serde(rename = "bridge.port")]
    pub bridge_port: u16,
    #[serde(rename = "updates.auto_check")]
    pub updates_auto_check: bool,
    pub instances_count: usize,
}
