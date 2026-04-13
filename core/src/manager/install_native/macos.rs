//! macOS-specific Node.js installation for Native mode.
//!
//! Downloads .pkg from nodejs.org, installs via `sudo installer`.
//! This requires admin password (macOS system prompt, not a terminal window).

use anyhow::Result;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{InstallProgress, InstallStage, send, has_node};

pub async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    send(tx, "Downloading Node.js for macOS...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0"; // LTS
    let url = format!("{nodejs_dist_base}/{version}/node-{version}-darwin-{arch}.pkg");
    let pkg_path = "/tmp/clawenv-node.pkg";

    // Download
    let status = Command::new("curl")
        .args(["-fSL", "-o", pkg_path, &url])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to download Node.js from {url}");
    }

    send(tx, "Installing Node.js (may require admin password)...", 18, InstallStage::EnsurePrerequisites).await;

    // Install via macOS installer (triggers admin password dialog)
    let status = Command::new("sudo")
        .args(["installer", "-pkg", pkg_path, "-target", "/"])
        .status()
        .await?;

    // Cleanup
    tokio::fs::remove_file(pkg_path).await.ok();

    if !status.success() {
        anyhow::bail!("Node.js installation failed. Please install manually from https://nodejs.org");
    }

    if !has_node().await {
        anyhow::bail!("Node.js installed but not found in PATH. Please restart the application.");
    }

    Ok(())
}
