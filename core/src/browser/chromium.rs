use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::sandbox::SandboxBackend;
use super::{BrowserBackend, BrowserStatus};

/// Chromium browser running inside a sandbox
pub struct ChromiumBackend {
    sandbox: Arc<dyn SandboxBackend>,
    cdp_port: u16,
    vnc_ws_port: u16,
}

impl ChromiumBackend {
    pub fn new(sandbox: Arc<dyn SandboxBackend>) -> Self {
        Self {
            sandbox,
            cdp_port: 9222,
            vnc_ws_port: 6080,
        }
    }
}

#[async_trait]
impl BrowserBackend for ChromiumBackend {
    async fn start_headless(&self, cdp_port: u16) -> Result<()> {
        // Launch Chromium in headless mode inside the sandbox
        self.sandbox.exec(&format!(
            "chromium-browser \
             --headless \
             --no-sandbox \
             --disable-gpu \
             --remote-debugging-address=0.0.0.0 \
             --remote-debugging-port={cdp_port} \
             --disable-dev-shm-usage &"
        )).await?;
        tracing::info!("Chromium headless started on CDP port {cdp_port}");
        Ok(())
    }

    async fn start_interactive(&self, vnc_ws_port: u16) -> Result<String> {
        // Stop headless if running
        self.sandbox.exec("pkill -f 'chromium.*headless'").await.ok();

        // Start Xvfb + VNC + noVNC chain
        self.sandbox.exec(
            "Xvfb :99 -screen 0 1280x720x24 &"
        ).await?;

        // Brief pause for Xvfb to initialize
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        self.sandbox.exec(
            "x11vnc -display :99 -nopw -listen 127.0.0.1 -port 5900 -shared -forever &"
        ).await?;

        self.sandbox.exec(&format!(
            "websockify --web=/usr/share/novnc {vnc_ws_port} 127.0.0.1:5900 &"
        )).await?;

        // Launch Chromium with display
        self.sandbox.exec(
            "DISPLAY=:99 chromium-browser --no-sandbox --disable-gpu &"
        ).await?;

        let novnc_url = format!("http://127.0.0.1:{vnc_ws_port}/vnc.html?autoconnect=true");
        tracing::info!("noVNC interactive mode started at {novnc_url}");
        Ok(novnc_url)
    }

    async fn resume_headless(&self) -> Result<()> {
        // Kill interactive chain
        self.sandbox.exec("pkill -f chromium").await.ok();
        self.sandbox.exec("pkill -f x11vnc").await.ok();
        self.sandbox.exec("pkill -f websockify").await.ok();
        self.sandbox.exec("pkill -f Xvfb").await.ok();

        // Restart in headless mode
        self.start_headless(self.cdp_port).await
    }

    async fn stop(&self) -> Result<()> {
        self.sandbox.exec("pkill -f chromium").await.ok();
        self.sandbox.exec("pkill -f x11vnc").await.ok();
        self.sandbox.exec("pkill -f websockify").await.ok();
        self.sandbox.exec("pkill -f Xvfb").await.ok();
        tracing::info!("Browser stopped");
        Ok(())
    }

    async fn status(&self) -> Result<BrowserStatus> {
        // Check if noVNC is running (interactive mode)
        let novnc = self.sandbox.exec("pgrep -f websockify").await;
        if let Ok(out) = &novnc {
            if !out.trim().is_empty() {
                return Ok(BrowserStatus::Interactive {
                    novnc_url: format!("http://127.0.0.1:{}/vnc.html?autoconnect=true", self.vnc_ws_port),
                });
            }
        }

        // Check if headless chromium is running
        let headless = self.sandbox.exec("pgrep -f 'chromium.*headless'").await;
        if let Ok(out) = &headless {
            if !out.trim().is_empty() {
                return Ok(BrowserStatus::Headless { cdp_port: self.cdp_port });
            }
        }

        Ok(BrowserStatus::Stopped)
    }
}
