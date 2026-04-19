use serde::{Deserialize, Serialize};
use crate::bridge::permissions::BridgePermissions;
use crate::sandbox::SandboxType;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub clawenv: ClawEnvConfig,
    #[serde(default)]
    pub instances: Vec<InstanceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawEnvConfig {
    pub version: String,
    #[serde(default = "default_user_mode")]
    pub user_mode: UserMode,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub updates: UpdateConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub tray: TrayConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub mirrors: MirrorsConfig,
    #[serde(default)]
    pub bridge: BridgeConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserMode {
    General,
    Developer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    #[serde(default = "default_true")]
    pub auto_check: bool,
    #[serde(default = "default_24")]
    pub check_interval_hours: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_keychain_backend")]
    pub keychain_backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrayConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub start_minimized: bool,
    #[serde(default = "default_true")]
    pub show_notifications: bool,
    #[serde(default = "default_5")]
    pub monitor_interval_sec: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub http_proxy: String,
    #[serde(default)]
    pub https_proxy: String,
    #[serde(default = "default_no_proxy")]
    pub no_proxy: String,
    #[serde(default)]
    pub auth_required: bool,
    #[serde(default)]
    pub auth_user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub name: String,
    #[serde(default = "default_claw_type")]
    pub claw_type: String,
    pub version: String,
    pub sandbox_type: SandboxType,
    #[serde(default)]
    pub sandbox_id: String,
    pub created_at: String,
    #[serde(default)]
    pub last_upgraded_at: String,
    #[serde(default, alias = "openclaw")]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub resources: ResourceConfig,
    #[serde(default)]
    pub browser: BrowserConfig,
    /// Cached latest version info (from last npm registry check)
    #[serde(default)]
    pub cached_latest_version: String,
    #[serde(default)]
    pub cached_version_check_at: String,
    /// Per-instance proxy config. `None` = inherit global `clawenv.proxy`
    /// from config.toml. Set to `Some(...)` via the ClawPage proxy modal
    /// when the user wants a different proxy for this specific instance
    /// (typical case: exported from machine A, imported on machine B with
    /// a different network).
    #[serde(default)]
    pub proxy: Option<InstanceProxyConfig>,
}

/// Per-instance proxy setting. Distinct from the global `ProxyConfig`
/// because the user's intent is different: global is "default for all
/// new installs"; per-instance is "this specific VM needs X, regardless
/// of what the host has globally".
///
/// `mode` = None/SyncHost/Manual. For `SyncHost`, `http_proxy` holds
/// the last-detected host proxy URL at apply-time (rewritten with the
/// backend-specific host IP, e.g. `http://host.lima.internal:7890`).
/// We store it so the config.toml is self-documenting — a user reading
/// the file can see what's actually being used without re-running detect.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstanceProxyConfig {
    #[serde(default = "default_instance_proxy_mode")]
    pub mode: String, // "none" | "sync-host" | "manual"
    #[serde(default)]
    pub http_proxy: String,
    #[serde(default)]
    pub https_proxy: String,
    #[serde(default = "default_no_proxy")]
    pub no_proxy: String,
    /// `true` when the proxy requires HTTP basic auth. Password lives in
    /// keychain (`proxy-password-<instance_name>`), never in config.toml.
    #[serde(default)]
    pub auth_required: bool,
    #[serde(default)]
    pub auth_user: String,
}

fn default_instance_proxy_mode() -> String { "none".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_gateway_port")]
    pub gateway_port: u16,
    #[serde(default = "default_ttyd_port")]
    pub ttyd_port: u16,
    /// Per-instance MCP bridge port (gateway_port + 2)
    #[serde(default)]
    pub bridge_port: u16,
    /// Per-instance web dashboard port, for claws that serve their
    /// management UI from a process separate from the gateway (e.g.
    /// Hermes: `hermes dashboard` on +5, independent of `hermes gateway`
    /// which is the OpenAI-compatible API). `0` means "no dashboard;
    /// the UI button opens gateway_port instead" — that's the case for
    /// OpenClaw and any older config.toml imported from pre-v0.2.7.
    #[serde(default)]
    pub dashboard_port: u16,
    #[serde(default)]
    pub webchat_enabled: bool,
    #[serde(default)]
    pub channels: ChannelsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConfig {
    #[serde(default = "default_512")]
    pub memory_limit_mb: u32,
    #[serde(default = "default_2")]
    pub cpu_cores: u32,
    #[serde(default = "default_workspace_path")]
    pub workspace_path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram_enabled: bool,
    #[serde(default)]
    pub whatsapp_enabled: bool,
    #[serde(default)]
    pub discord_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_browser_mode")]
    pub mode: String,  // "headless" | "fingerprint" | "host-cdp"
    #[serde(default = "default_cdp_port")]
    pub cdp_port: u16,
    #[serde(default = "default_vnc_port")]
    pub vnc_ws_port: u16,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_browser_mode(),
            cdp_port: default_cdp_port(),
            vnc_ws_port: default_vnc_port(),
        }
    }
}

fn default_browser_mode() -> String { "headless".into() }
fn default_cdp_port() -> u16 { 9222 }
fn default_vnc_port() -> u16 { 6080 }
fn default_workspace_path() -> String { "~/.clawenv/workspaces/default".into() }

// Default value helpers
fn default_user_mode() -> UserMode { UserMode::General }
fn default_language() -> String { "zh-CN".into() }
fn default_theme() -> String { "system".into() }
fn default_true() -> bool { true }
fn default_24() -> u32 { 24 }
fn default_5() -> u32 { 5 }
fn default_512() -> u32 { 512 }
fn default_2() -> u32 { 2 }
fn default_keychain_backend() -> String { "system".into() }
fn default_no_proxy() -> String { "localhost,127.0.0.1".into() }
fn default_claw_type() -> String { "openclaw".into() }
fn default_gateway_port() -> u16 { 3000 }
fn default_ttyd_port() -> u16 { 3001 }

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            auto_check: true,
            check_interval_hours: 24,
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self { keychain_backend: "system".into() }
    }
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            start_minimized: false,
            show_notifications: true,
            monitor_interval_sec: 5,
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            http_proxy: String::new(),
            https_proxy: String::new(),
            no_proxy: "localhost,127.0.0.1".into(),
            auth_required: false,
            auth_user: String::new(),
        }
    }
}

/// Mirror configuration for package sources.
/// Supports presets ("default", "china") or fully custom URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MirrorsConfig {
    /// Preset: "default" (official sources), "china" (domestic mirrors), "custom"
    #[serde(default = "default_mirror_preset")]
    pub preset: String,
    /// Alpine APK repository base URL (e.g., "https://mirrors.aliyun.com/alpine")
    #[serde(default)]
    pub alpine_repo: String,
    /// npm registry URL (e.g., "https://registry.npmmirror.com")
    #[serde(default)]
    pub npm_registry: String,
    /// Node.js binary download base URL (e.g., "https://npmmirror.com/mirrors/node")
    #[serde(default)]
    pub nodejs_dist: String,
}

fn default_mirror_preset() -> String { "default".into() }

impl MirrorsConfig {
    /// Resolve effective URLs: if preset is "china" or "default", use known URLs;
    /// if "custom", use the user-provided fields.
    pub fn alpine_repo_url(&self) -> &str {
        if !self.alpine_repo.is_empty() { return &self.alpine_repo; }
        match self.preset.as_str() {
            "china" => "https://mirrors.aliyun.com/alpine",
            _ => "https://dl-cdn.alpinelinux.org/alpine",
        }
    }

    pub fn npm_registry_url(&self) -> &str {
        if !self.npm_registry.is_empty() { return &self.npm_registry; }
        match self.preset.as_str() {
            "china" => "https://registry.npmmirror.com",
            _ => "https://registry.npmjs.org",
        }
    }

    pub fn nodejs_dist_url(&self) -> &str {
        if !self.nodejs_dist.is_empty() { return &self.nodejs_dist; }
        match self.preset.as_str() {
            "china" => "https://npmmirror.com/mirrors/node",
            _ => "https://nodejs.org/dist",
        }
    }

    pub fn is_default(&self) -> bool {
        self.preset == "default" && self.alpine_repo.is_empty()
            && self.npm_registry.is_empty() && self.nodejs_dist.is_empty()
    }
}

impl Default for MirrorsConfig {
    fn default() -> Self {
        Self {
            preset: "default".into(),
            alpine_repo: String::new(),
            npm_registry: String::new(),
            nodejs_dist: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_bridge_port")]
    pub port: u16,
    #[serde(default)]
    pub permissions: BridgePermissions,
}

fn default_bridge_port() -> u16 { 3100 }

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: default_bridge_port(),
            permissions: BridgePermissions::default(),
        }
    }
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            gateway_port: 3000,
            ttyd_port: default_ttyd_port(),
            bridge_port: 3002,
            // 0 = no dashboard. Real installs call allocate_port when the
            // claw descriptor has dashboard_cmd; Default is used for
            // synthetic / test / import-fallback cases where there's no
            // descriptor context to consult.
            dashboard_port: 0,
            webchat_enabled: false,
            channels: ChannelsConfig::default(),
        }
    }
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            memory_limit_mb: 512,
            cpu_cores: 2,
            workspace_path: default_workspace_path(),
        }
    }
}
