pub mod chromium;
pub mod cdp;
pub mod fingerprint;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserStatus {
    Stopped,
    Headless { cdp_port: u16 },
    Interactive { novnc_url: String },
}

/// Browser backend trait — independent of SandboxBackend
#[async_trait]
pub trait BrowserBackend: Send + Sync {
    /// Start Chromium in headless mode
    async fn start_headless(&self, cdp_port: u16) -> Result<()>;

    /// Switch to interactive mode (noVNC) for human intervention
    async fn start_interactive(&self, vnc_ws_port: u16) -> Result<String>;

    /// Switch back to headless mode
    async fn resume_headless(&self) -> Result<()>;

    /// Stop the browser
    async fn stop(&self) -> Result<()>;

    /// Get current browser status
    async fn status(&self) -> Result<BrowserStatus>;
}
