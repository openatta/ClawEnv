// Phase 4: Fingerprint browser (Camoufox) integration
// Anti-detection browser for advanced web automation scenarios.
// Planned for Phase 4 implementation.

use anyhow::Result;
use async_trait::async_trait;
use super::{BrowserBackend, BrowserStatus};

pub struct FingerprintBrowserBackend;

#[async_trait]
impl BrowserBackend for FingerprintBrowserBackend {
    async fn start_headless(&self, _cdp_port: u16) -> Result<()> {
        anyhow::bail!("Fingerprint browser is planned for Phase 4")
    }
    async fn start_interactive(&self, _vnc_ws_port: u16) -> Result<String> {
        anyhow::bail!("Fingerprint browser is planned for Phase 4")
    }
    async fn resume_headless(&self) -> Result<()> {
        anyhow::bail!("Fingerprint browser is planned for Phase 4")
    }
    async fn stop(&self) -> Result<()> { Ok(()) }
    async fn status(&self) -> Result<BrowserStatus> {
        Ok(BrowserStatus::Stopped)
    }
}
