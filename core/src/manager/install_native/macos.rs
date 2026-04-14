//! macOS-specific Node.js installation for Native mode.
//!
//! Downloads .tar.gz from nodejs.org, extracts to ~/.clawenv/node/.
//! No admin privileges needed — fully self-contained in user directory.

use anyhow::Result;
use tokio::sync::mpsc;

use super::{InstallProgress, InstallStage, send, clawenv_node_dir, ensure_node_in_path};

pub async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    send(tx, "Downloading Node.js for macOS...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0";
    let url = format!("{nodejs_dist_base}/{version}/node-{version}-darwin-{arch}.tar.gz");

    let node_dir = clawenv_node_dir();
    tokio::fs::create_dir_all(&node_dir).await?;

    let tar_path = node_dir.parent().unwrap_or(&node_dir).join("node.tar.gz");

    let status = tokio::process::Command::new("curl")
        .args(["-fSL", "-o", &tar_path.to_string_lossy(), &url])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to download Node.js from {url}");
    }

    send(tx, "Extracting Node.js...", 18, InstallStage::EnsurePrerequisites).await;

    // Extract tar.gz, strip the top-level directory
    let status = tokio::process::Command::new("tar")
        .args(["xzf", &tar_path.to_string_lossy(), "--strip-components=1", "-C", &node_dir.to_string_lossy()])
        .status()
        .await?;

    // Cleanup
    tokio::fs::remove_file(&tar_path).await.ok();

    if !status.success() {
        anyhow::bail!("Failed to extract Node.js");
    }

    send(tx, "Node.js installed to ~/.clawenv/node", 22, InstallStage::EnsurePrerequisites).await;
    ensure_node_in_path();

    Ok(())
}
