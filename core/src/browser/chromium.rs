//! Chromium browser running inside a sandbox VM. Lifted verbatim
//! from v1 `core/src/browser/chromium.rs` (P1-e). Function bodies
//! are unchanged — only imports + exec→exec_argv where command is
//! a constant.

use std::sync::Arc;

use async_trait::async_trait;

use crate::sandbox_backend::SandboxBackend;
use super::{BrowserBackend, BrowserStatus};

/// Chromium backend. Wraps a SandboxBackend; all commands run via
/// `sandbox.exec_argv(&["sh", "-c", ...])` inside the VM.
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

    pub fn with_ports(sandbox: Arc<dyn SandboxBackend>, cdp_port: u16, vnc_ws_port: u16) -> Self {
        Self { sandbox, cdp_port, vnc_ws_port }
    }
}

#[async_trait]
impl BrowserBackend for ChromiumBackend {
    async fn start_headless(&self, cdp_port: u16) -> anyhow::Result<()> {
        let cmd = format!(
            "chromium-browser \
             --headless \
             --no-sandbox \
             --disable-gpu \
             --remote-debugging-address=0.0.0.0 \
             --remote-debugging-port={cdp_port} \
             --disable-dev-shm-usage &"
        );
        self.sandbox.exec_argv(&["sh", "-c", &cmd]).await?;
        tracing::info!("Chromium headless started on CDP port {cdp_port}");
        Ok(())
    }

    async fn start_interactive(&self, vnc_ws_port: u16) -> anyhow::Result<String> {
        // Stop headless if running.
        let _ = self.sandbox
            .exec_argv(&["sh", "-c", "pkill -f 'chromium.*headless'"])
            .await;

        // Bring up Xvfb → x11vnc → websockify → chromium chain.
        self.sandbox.exec_argv(&[
            "sh", "-c", "Xvfb :99 -screen 0 1280x720x24 &"
        ]).await?;

        // Brief pause for Xvfb to initialize before clients connect.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        self.sandbox.exec_argv(&[
            "sh", "-c",
            "x11vnc -display :99 -nopw -listen 127.0.0.1 -port 5900 -shared -forever &",
        ]).await?;

        let websockify_cmd = format!(
            "websockify --web=/usr/share/novnc {vnc_ws_port} 127.0.0.1:5900 &"
        );
        self.sandbox.exec_argv(&["sh", "-c", &websockify_cmd]).await?;

        self.sandbox.exec_argv(&[
            "sh", "-c", "DISPLAY=:99 chromium-browser --no-sandbox --disable-gpu &"
        ]).await?;

        let novnc_url = format!("http://127.0.0.1:{vnc_ws_port}/vnc.html?autoconnect=true");
        tracing::info!("noVNC interactive mode started at {novnc_url}");
        Ok(novnc_url)
    }

    async fn resume_headless(&self) -> anyhow::Result<()> {
        for pat in ["chromium", "x11vnc", "websockify", "Xvfb"] {
            let _ = self.sandbox
                .exec_argv(&["sh", "-c", &format!("pkill -f {pat}")])
                .await;
        }
        self.start_headless(self.cdp_port).await
    }

    async fn stop(&self) -> anyhow::Result<()> {
        for pat in ["chromium", "x11vnc", "websockify", "Xvfb"] {
            let _ = self.sandbox
                .exec_argv(&["sh", "-c", &format!("pkill -f {pat}")])
                .await;
        }
        tracing::info!("Browser stopped");
        Ok(())
    }

    async fn status(&self) -> anyhow::Result<BrowserStatus> {
        // noVNC running → interactive mode wins.
        let novnc = self.sandbox
            .exec_argv(&["sh", "-c", "pgrep -f websockify"])
            .await;
        if let Ok(out) = &novnc {
            if !out.trim().is_empty() {
                return Ok(BrowserStatus::Interactive {
                    novnc_url: format!(
                        "http://127.0.0.1:{}/vnc.html?autoconnect=true",
                        self.vnc_ws_port
                    ),
                });
            }
        }
        // Headless chromium check.
        let headless = self.sandbox
            .exec_argv(&["sh", "-c", "pgrep -f 'chromium.*headless'"])
            .await;
        if let Ok(out) = &headless {
            if !out.trim().is_empty() {
                return Ok(BrowserStatus::Headless { cdp_port: self.cdp_port });
            }
        }
        Ok(BrowserStatus::Stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_ops::testing::MockBackend;

    fn arc_mock(stdout: &str) -> Arc<dyn SandboxBackend> {
        Arc::new(MockBackend::new("fake").with_stdout(stdout))
    }

    #[tokio::test]
    async fn status_stopped_when_nothing_running() {
        // Both pgrep calls return empty stdout = nothing running.
        let backend = arc_mock("");
        let bb = ChromiumBackend::new(backend);
        let s = bb.status().await.unwrap();
        assert!(matches!(s, BrowserStatus::Stopped));
    }

    #[tokio::test]
    async fn status_interactive_when_websockify_pgrep_returns_pid() {
        // First exec call returns a PID → Interactive wins.
        let backend = arc_mock("12345\n");
        let bb = ChromiumBackend::new(backend);
        let s = bb.status().await.unwrap();
        match s {
            BrowserStatus::Interactive { novnc_url } => {
                assert!(novnc_url.contains("/vnc.html"));
                assert!(novnc_url.contains("6080"));
            }
            other => panic!("expected Interactive, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_headless_emits_chromium_command() {
        let mock = Arc::new(MockBackend::new("fake"));
        let backend: Arc<dyn SandboxBackend> = mock.clone();
        let bb = ChromiumBackend::new(backend);
        bb.start_headless(9222).await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("chromium-browser"));
        assert!(log[0].contains("--headless"));
        assert!(log[0].contains("--remote-debugging-port=9222"));
    }

    #[tokio::test]
    async fn start_interactive_brings_up_full_chain() {
        let mock = Arc::new(MockBackend::new("fake"));
        let backend: Arc<dyn SandboxBackend> = mock.clone();
        let bb = ChromiumBackend::new(backend);
        let url = bb.start_interactive(6080).await.unwrap();
        assert!(url.contains("6080"));
        let log = mock.exec_log.lock().unwrap();
        // pkill (1) + Xvfb (1) + x11vnc (1) + websockify (1) + chromium (1) = 5.
        assert_eq!(log.len(), 5, "expected 5 chain calls: {log:?}");
        let joined = log.join("\n");
        assert!(joined.contains("Xvfb"));
        assert!(joined.contains("x11vnc"));
        assert!(joined.contains("websockify"));
        assert!(joined.contains("DISPLAY=:99"));
    }

    #[tokio::test]
    async fn stop_kills_all_processes() {
        let mock = Arc::new(MockBackend::new("fake"));
        let backend: Arc<dyn SandboxBackend> = mock.clone();
        let bb = ChromiumBackend::new(backend);
        bb.stop().await.unwrap();
        let log = mock.exec_log.lock().unwrap();
        assert_eq!(log.len(), 4, "expected 4 pkill calls: {log:?}");
        for line in log.iter() {
            assert!(line.contains("pkill"), "non-pkill cmd: {line}");
        }
    }
}
