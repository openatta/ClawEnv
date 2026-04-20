//! Linux-specific Node.js installation for Native mode.
//!
//! Downloads .tar.xz from nodejs.org, extracts to ~/.clawenv/node/ using tar.
//! No root privileges needed — fully self-contained in user directory.

use anyhow::Result;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{InstallProgress, InstallStage, send, clawenv_node_dir, ensure_node_in_path, has_node};

pub async fn install_nodejs(
    tx: &mpsc::Sender<InstallProgress>,
    nodejs_dist_base: &str,
    _proxy_on: bool,
) -> Result<()> {
    // Linux Native is the "developer mode" single-URL path. Proxy-aware
    // fallback on Linux is not wired up yet (the curl|tar pipeline here
    // doesn't use download_with_progress). Taking proxy_on via the
    // argument so the signature matches the unix/windows/macos trio;
    // bringing Linux up to parity is tracked as follow-up.
    send(tx, "Downloading Node.js for Linux...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0";
    let url = format!("{nodejs_dist_base}/{version}/node-{version}-linux-{arch}.tar.xz");

    // Install to ~/.clawenv/node/ (no root needed)
    let node_dir = clawenv_node_dir();
    tokio::fs::create_dir_all(&node_dir).await?;

    let status = Command::new("sh")
        .args(["-c", &format!(
            "curl -fSL '{url}' | tar -xJ --strip-components=1 -C '{}'",
            node_dir.display()
        )])
        .status()
        .await?;

    if !status.success() {
        anyhow::bail!("Failed to download and extract Node.js");
    }

    send(tx, "Node.js installed to ~/.clawenv/node", 22, InstallStage::EnsurePrerequisites).await;

    // Add to PATH for this process
    ensure_node_in_path();

    if !has_node().await {
        let bin_path = node_dir.join("bin");
        anyhow::bail!(
            "Node.js installed to {} but not reachable. Add to PATH: export PATH=\"{}:$PATH\"",
            node_dir.display(), bin_path.display()
        );
    }

    Ok(())
}
