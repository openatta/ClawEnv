//! Browser HIL (Human In the Loop) — Chromium running inside a
//! sandbox VM with optional noVNC takeover for human review.
//!
//! Lifted from v1 `core/src/browser/{mod.rs, chromium.rs}` (P1-e).
//! v1 also has `cdp.rs` (Phase-2 CDP relay) and `fingerprint.rs`
//! (Phase-4 Camoufox) — both bail-only stubs in v1; not ported here
//! until they're real.
//!
//! Three modes per claw:
//! - **Headless**: Chromium runs with `--headless --remote-debugging-port`,
//!   no display, agent drives via CDP. Default for OpenClaw.
//! - **Interactive (HIL)**: when the agent hits a CAPTCHA or 2FA, it
//!   pauses, ClawEnv switches to Xvfb + x11vnc + websockify chain
//!   so the user can drive Chromium via noVNC in their browser.
//! - **Stopped**: nothing running.
//!
//! Required apk packages inside the VM: `chromium xvfb-run x11vnc
//! novnc websockify ttf-freefont` — installed by
//! [`install_browser_packages`] (or at provision time when the user
//! passes `--install-browser` to install).

pub mod chromium;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::common::{OpsError, ProgressSink};
use crate::provisioning::{run_background_script, BackgroundScriptOpts};
use crate::sandbox_backend::SandboxBackend;

pub use chromium::ChromiumBackend;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserStatus {
    Stopped,
    Headless { cdp_port: u16 },
    Interactive { novnc_url: String },
}

/// Browser backend trait. Independent of `SandboxBackend` — the
/// concrete `ChromiumBackend` consumes a `SandboxBackend` to run its
/// commands inside the VM, but the trait surface itself is just the
/// HIL state machine.
#[async_trait]
pub trait BrowserBackend: Send + Sync {
    /// Start Chromium in headless mode (CDP-driven by the agent).
    async fn start_headless(&self, cdp_port: u16) -> anyhow::Result<()>;

    /// Switch to interactive mode (noVNC) for human takeover.
    /// Returns the noVNC URL the user should open.
    async fn start_interactive(&self, vnc_ws_port: u16) -> anyhow::Result<String>;

    /// Switch back to headless after the user finishes their HIL action.
    async fn resume_headless(&self) -> anyhow::Result<()>;

    /// Stop all browser processes.
    async fn stop(&self) -> anyhow::Result<()>;

    /// What state are we in right now?
    async fn status(&self) -> anyhow::Result<BrowserStatus>;
}

/// Install the apk packages needed for the Chromium HIL stack inside
/// the given sandbox. Long-running (~5-10 min on slow networks) so it
/// streams progress through `progress`. Idempotent — `apk add` is a
/// no-op when packages are present.
pub async fn install_browser_packages(
    backend: &Arc<dyn SandboxBackend>,
    progress: &ProgressSink,
) -> Result<(), OpsError> {
    let cmd = "sudo apk add --no-cache \
                 chromium xvfb-run x11vnc novnc websockify ttf-freefont";
    let opts = BackgroundScriptOpts {
        cmd,
        label: "Browser HIL packages",
        sudo: false, // command itself uses sudo; bg script doesn't need to wrap
        log_file: "/tmp/clawenv-browser-install.log",
        done_file: "/tmp/clawenv-browser-install.done",
        script_file: "/tmp/clawenv-browser-install.sh",
        pct_range: (10, 95),
        ..Default::default()
    };
    run_background_script(backend, &opts, progress).await?;
    Ok(())
}
