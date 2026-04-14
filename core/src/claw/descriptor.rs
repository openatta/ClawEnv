//! Claw product descriptor — defines how to install, run, and manage a specific claw variant.
//!
//! Each claw product (OpenClaw, ZeroClaw, AutoClaw, etc.) has different:
//!   - npm package names
//!   - CLI binary names and command syntax
//!   - Default ports
//!   - Feature support (MCP, browser, etc.)
//!
//! The descriptor abstracts these differences so the install/upgrade/instance
//! management code never hardcodes "openclaw".

use serde::{Deserialize, Serialize};

/// Describes a specific claw product and how to interact with it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawDescriptor {
    /// Unique identifier, used as key in registry (e.g., "openclaw", "zeroclaw")
    pub id: String,
    /// Human-readable display name (e.g., "OpenClaw", "智谱 AutoClaw")
    pub display_name: String,
    /// Logo: emoji string or relative path to SVG in assets/logos/ (e.g., "🦞" or "logos/autoclaw.svg")
    #[serde(default)]
    pub logo: String,

    // ---- Installation ----
    /// npm package name (e.g., "openclaw", "@zhipu/autoclaw")
    pub npm_package: String,

    // ---- CLI interface ----
    /// Binary name after `npm install -g` (e.g., "openclaw", "zeroclaw")
    pub cli_binary: String,
    /// Command to start the gateway/server, with `{port}` placeholder
    /// e.g., "gateway --port {port} --allow-unconfigured"
    pub gateway_cmd: String,
    /// Command to check version (e.g., "--version")
    pub version_cmd: String,
    /// Command to set API key, with `{key}` placeholder
    /// e.g., "config set apiKey '{key}'"
    /// Empty string means this claw doesn't support API key configuration via CLI.
    #[serde(default)]
    pub config_apikey_cmd: String,
    /// Command to register an MCP server, with `{name}` and `{json}` placeholders
    /// e.g., "mcp set {name} '{json}'"
    /// Empty string means MCP is not supported.
    #[serde(default)]
    pub mcp_set_cmd: String,

    // ---- Defaults ----
    /// Default gateway port
    #[serde(default = "default_port")]
    pub default_port: u16,

    // ---- Feature flags ----
    /// Whether this claw supports MCP Bridge integration
    #[serde(default)]
    pub supports_mcp: bool,
    /// Whether this claw supports browser automation (Chromium in sandbox)
    #[serde(default)]
    pub supports_browser: bool,
}

fn default_port() -> u16 { 3000 }

impl ClawDescriptor {
    /// Format the gateway start command with the actual port.
    pub fn gateway_start_cmd(&self, port: u16) -> String {
        format!(
            "{} {}",
            self.cli_binary,
            self.gateway_cmd.replace("{port}", &port.to_string())
        )
    }

    /// Format the version check command.
    pub fn version_check_cmd(&self) -> String {
        format!("{} {}", self.cli_binary, self.version_cmd)
    }

    /// Format the API key configuration command. Returns None if not supported.
    pub fn set_apikey_cmd(&self, key: &str) -> Option<String> {
        if self.config_apikey_cmd.is_empty() {
            return None;
        }
        Some(format!(
            "{} {}",
            self.cli_binary,
            self.config_apikey_cmd.replace("{key}", key)
        ))
    }

    /// Format the MCP set command. Returns None if not supported.
    pub fn mcp_register_cmd(&self, name: &str, json: &str) -> Option<String> {
        if self.mcp_set_cmd.is_empty() {
            return None;
        }
        Some(format!(
            "{} {}",
            self.cli_binary,
            self.mcp_set_cmd
                .replace("{name}", name)
                .replace("{json}", json)
        ))
    }

    /// The npm install command string.
    pub fn npm_install_cmd(&self, version: &str) -> String {
        format!("npm install -g {}@{}", self.npm_package, version)
    }

    /// The npm install command with verbose logging (for progress tracking).
    pub fn npm_install_verbose_cmd(&self, version: &str) -> String {
        format!("npm install -g --loglevel verbose {}@{}", self.npm_package, version)
    }

    /// Process name patterns for kill commands.
    /// Returns both "binary gateway" and "binary-gateway" to match
    /// different process naming conventions.
    pub fn process_names(&self) -> Vec<String> {
        vec![
            format!("{} gateway", self.cli_binary),
            format!("{}-gateway", self.cli_binary),
        ]
    }
}

/// Built-in OpenClaw descriptor (the default).
pub fn openclaw_descriptor() -> ClawDescriptor {
    ClawDescriptor {
        id: "openclaw".into(),
        display_name: "OpenClaw".into(),
        logo: "🦞".into(),
        npm_package: "openclaw".into(),
        cli_binary: "openclaw".into(),
        gateway_cmd: "gateway --port {port} --allow-unconfigured".into(),
        version_cmd: "--version".into(),
        config_apikey_cmd: "config set apiKey '{key}'".into(),
        mcp_set_cmd: "mcp set {name} '{json}'".into(),
        default_port: 3000,
        supports_mcp: true,
        supports_browser: true,
    }
}
