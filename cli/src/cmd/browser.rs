//! `clawcli browser ...` — Chromium HIL state machine surface.
//!
//! Mirrors the Tauri IPCs `browser_status` / `browser_start_interactive`
//! / `browser_resume_headless` so the GUI can switch entirely to
//! cli_bridge dispatch (Phase M, thin-shell strategy).
//!
//! All subcommands target an instance by name (positional, falls back
//! to the `--instance` global flag). The instance must be a sandboxed
//! one — native installs don't run a Chromium HIL stack.

use std::sync::Arc;

use clap::Subcommand;
use clawops_core::browser::{BrowserBackend, BrowserStatus, ChromiumBackend};
use clawops_core::instance::{InstanceConfig, InstanceRegistry, SandboxKind};
use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};

use crate::shared::Ctx;

#[derive(Subcommand)]
pub enum BrowserCmd {
    /// Report current browser state (Stopped / Headless / Interactive).
    Status { name: Option<String> },
    /// Switch from headless → noVNC HIL mode for human takeover.
    /// Returns the noVNC URL the user should open in their browser.
    HilStart { name: Option<String> },
    /// Switch back from noVNC → headless after the user finishes HIL.
    HilResume { name: Option<String> },
}

pub async fn run(cmd: BrowserCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        BrowserCmd::Status { name } => {
            let bb = build_browser(name, ctx).await?;
            let s = bb.status().await?;
            ctx.emit_pretty(&s, |st| match st {
                BrowserStatus::Stopped => println!("state: stopped"),
                BrowserStatus::Headless { cdp_port } =>
                    println!("state: headless\ncdp_port: {cdp_port}"),
                BrowserStatus::Interactive { novnc_url } =>
                    println!("state: interactive\nnovnc_url: {novnc_url}"),
            })?;
        }
        BrowserCmd::HilStart { name } => {
            let bb = build_browser(name, ctx).await?;
            // Default vnc_ws_port — same as P1-e ChromiumBackend default
            // (6080). If users need a different port we'll add a flag,
            // but the GUI doesn't customise this today.
            let url = bb.start_interactive(6080).await?;
            ctx.emit_pretty(&url, |u| println!("noVNC: {u}"))?;
        }
        BrowserCmd::HilResume { name } => {
            let bb = build_browser(name, ctx).await?;
            bb.resume_headless().await?;
            ctx.emit_text("browser back to headless");
        }
    }
    Ok(())
}

async fn build_browser(
    name: Option<String>,
    ctx: &Ctx,
) -> anyhow::Result<ChromiumBackend> {
    let n = name.unwrap_or_else(|| ctx.instance.clone());
    let cfg = resolve_instance(&n).await?;
    if matches!(cfg.backend, SandboxKind::Native) {
        anyhow::bail!("instance `{n}` is native — Chromium HIL only available in sandbox VMs");
    }
    let target = if cfg.sandbox_instance.is_empty() {
        cfg.name.clone()
    } else {
        cfg.sandbox_instance.clone()
    };
    let backend: Arc<dyn SandboxBackend> = match cfg.backend {
        SandboxKind::Lima => Arc::new(LimaBackend::new(&target)),
        SandboxKind::Wsl2 => Arc::new(WslBackend::new(&target)),
        SandboxKind::Podman => Arc::new(PodmanBackend::new(&target)),
        SandboxKind::Native => unreachable!("checked above"),
    };
    Ok(ChromiumBackend::new(backend))
}

async fn resolve_instance(name: &str) -> anyhow::Result<InstanceConfig> {
    let reg = InstanceRegistry::with_default_path();
    reg.find(name).await?
        .ok_or_else(|| anyhow::anyhow!("instance `{name}` not found"))
}
