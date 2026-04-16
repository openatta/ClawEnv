//! Claw product descriptor — defines how to install, run, and manage a specific claw variant.
//!
//! Each claw product (OpenClaw, Hermes Agent, NanoClaw, etc.) has different:
//!   - Package managers (npm, pip) and install commands
//!   - CLI binary names and command syntax
//!   - Default ports
//!   - Feature support (MCP, browser, gateway UI, etc.)
//!
//! The descriptor abstracts these differences so the install/upgrade/instance
//! management code never hardcodes "openclaw".

use serde::{Deserialize, Serialize};

/// How a claw product is installed inside the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    /// Node.js / npm: `npm install -g <package>@<version>`
    Npm,
    /// Python / pip: `pip install <package>==<version>`
    Pip,
}

impl Default for PackageManager {
    fn default() -> Self { Self::Npm }
}

/// Describes a specific claw product and how to interact with it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClawDescriptor {
    /// Unique identifier, used as key in registry (e.g., "openclaw", "hermes")
    pub id: String,
    /// Human-readable display name (e.g., "OpenClaw", "Hermes Agent")
    pub display_name: String,
    /// Logo: emoji string or relative path to SVG in assets/logos/ (e.g., "🦞" or "logos/autoclaw.svg")
    #[serde(default)]
    pub logo: String,

    // ---- Installation ----
    /// Package manager: "npm" (default) or "pip"
    #[serde(default)]
    pub package_manager: PackageManager,
    /// npm package name (e.g., "openclaw") — used when package_manager = npm
    #[serde(default)]
    pub npm_package: String,
    /// pip package name (e.g., "hermes-agent") — used when package_manager = pip
    #[serde(default)]
    pub pip_package: String,
    /// Extra Alpine packages to install before the claw itself (e.g., ["python3", "py3-pip"])
    #[serde(default)]
    pub sandbox_provision: Vec<String>,

    // ---- CLI interface ----
    /// Binary name after install (e.g., "openclaw", "hermes")
    pub cli_binary: String,
    /// Command to start the gateway/server, with `{port}` placeholder
    /// e.g., "gateway --port {port} --allow-unconfigured"
    /// Empty string means this claw has no gateway/server mode.
    #[serde(default)]
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
    /// Whether this claw has a built-in web UI (gateway control panel)
    /// If false, ClawPage shows terminal (ttyd) instead of "Open Control Panel" button.
    #[serde(default = "default_true")]
    pub has_gateway_ui: bool,
    /// Whether native (non-sandbox) installation is supported
    #[serde(default = "default_true")]
    pub supports_native: bool,
    /// MCP bridge runtime language: "node" (default) or "python"
    /// Determines which bridge script (mcp-bridge.mjs vs mcp-bridge.py) to deploy.
    #[serde(default = "default_mcp_runtime")]
    pub mcp_runtime: String,
}

fn default_port() -> u16 { 3000 }
fn default_true() -> bool { true }
fn default_mcp_runtime() -> String { "node".into() }

impl ClawDescriptor {
    /// Format the gateway start command with the actual port.
    /// Returns None if this claw has no gateway mode.
    pub fn gateway_start_cmd(&self, port: u16) -> Option<String> {
        if self.gateway_cmd.is_empty() {
            return None;
        }
        Some(format!(
            "{} {}",
            self.cli_binary,
            self.gateway_cmd.replace("{port}", &port.to_string())
        ))
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

    /// Package install command (sandbox mode, verbose for progress tracking).
    /// Dispatches based on `package_manager`.
    pub fn sandbox_install_cmd(&self, version: &str) -> String {
        match self.package_manager {
            PackageManager::Npm => {
                format!("npm install -g --loglevel verbose {}@{}", self.npm_package, version)
            }
            PackageManager::Pip => {
                if version == "latest" {
                    format!("pip install --break-system-packages {}", self.pip_package)
                } else {
                    format!("pip install --break-system-packages {}=={}", self.pip_package, version)
                }
            }
        }
    }

    /// The npm install command string (kept for backwards compatibility with native mode).
    pub fn npm_install_cmd(&self, version: &str) -> String {
        format!("npm install -g {}@{}", self.npm_package, version)
    }

    /// The npm install command with verbose logging (for progress tracking).
    pub fn npm_install_verbose_cmd(&self, version: &str) -> String {
        format!("npm install -g --loglevel verbose {}@{}", self.npm_package, version)
    }

    /// npm install with --prefix for native mode (installs into instance dir).
    pub fn npm_install_prefix_cmd(&self, version: &str, prefix: &str) -> String {
        format!("npm install -g --prefix \"{}\" --loglevel verbose {}@{}", prefix, self.npm_package, version)
    }

    /// Process name patterns for kill commands.
    /// Returns both "binary gateway" and "binary-gateway" to match
    /// different process naming conventions.
    /// Does NOT include the bare binary name to avoid killing unrelated processes.
    pub fn process_names(&self) -> Vec<String> {
        vec![
            format!("{} gateway", self.cli_binary),
            format!("{}-gateway", self.cli_binary),
        ]
    }

    /// Whether MCP bridge scripts should use Python runtime (vs Node.js).
    pub fn uses_python_mcp(&self) -> bool {
        self.mcp_runtime.eq_ignore_ascii_case("python")
    }
}

/// Built-in OpenClaw descriptor (the default).
pub fn openclaw_descriptor() -> ClawDescriptor {
    ClawDescriptor {
        id: "openclaw".into(),
        display_name: "OpenClaw".into(),
        logo: "🦞".into(),
        package_manager: PackageManager::Npm,
        npm_package: "openclaw".into(),
        pip_package: String::new(),
        sandbox_provision: vec![],
        cli_binary: "openclaw".into(),
        gateway_cmd: "gateway --port {port} --allow-unconfigured".into(),
        version_cmd: "--version".into(),
        config_apikey_cmd: "config set apiKey '{key}'".into(),
        mcp_set_cmd: "mcp set {name} '{json}'".into(),
        default_port: 3000,
        supports_mcp: true,
        supports_browser: true,
        has_gateway_ui: true,
        supports_native: true,
        mcp_runtime: "node".into(),
    }
}
