// Phase 2: CDP Relay — attach to real browser tabs via Chrome DevTools Protocol
// This module will provide a relay between sandbox OpenClaw and host browser CDP port.
// Planned for Phase 2 implementation.

use anyhow::Result;
use async_trait::async_trait;
use super::{BrowserBackend, BrowserStatus};

pub struct CdpRelayBackend;

#[async_trait]
impl BrowserBackend for CdpRelayBackend {
    async fn start_headless(&self, _cdp_port: u16) -> Result<()> {
        anyhow::bail!("CDP Relay is planned for Phase 2")
    }
    async fn start_interactive(&self, _vnc_ws_port: u16) -> Result<String> {
        anyhow::bail!("CDP Relay is planned for Phase 2")
    }
    async fn resume_headless(&self) -> Result<()> {
        anyhow::bail!("CDP Relay is planned for Phase 2")
    }
    async fn stop(&self) -> Result<()> { Ok(()) }
    async fn status(&self) -> Result<BrowserStatus> {
        Ok(BrowserStatus::Stopped)
    }
}
