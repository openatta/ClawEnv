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
    #[serde(default)]
    pub openclaw: OpenClawConfig,
    #[serde(default)]
    pub resources: ResourceConfig,
    #[serde(default)]
    pub browser: BrowserConfig,
    /// Cached latest version info (from last npm registry check)
    #[serde(default)]
    pub cached_latest_version: String,
    #[serde(default)]
    pub cached_version_check_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenClawConfig {
    #[serde(default = "default_gateway_port")]
    pub gateway_port: u16,
    #[serde(default = "default_ttyd_port")]
    pub ttyd_port: u16,
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
fn default_ttyd_port() -> u16 { 7681 }

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
            enabled: false,
            port: default_bridge_port(),
            permissions: BridgePermissions::default(),
        }
    }
}

impl Default for OpenClawConfig {
    fn default() -> Self {
        Self {
            gateway_port: 3000,
            ttyd_port: default_ttyd_port(),
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
