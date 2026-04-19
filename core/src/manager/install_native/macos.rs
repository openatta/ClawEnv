//! macOS-specific Node.js installation for Native mode.
//!
//! Streams the .tar.gz from nodejs.org (with CN mirror fallback) and
//! extracts into ~/.clawenv/node/. No admin privileges — fully self-
//! contained in the user directory.

use anyhow::Result;
use tokio::sync::mpsc;

use super::{
    build_node_urls, clawenv_node_dir, ensure_node_in_path, send,
    InstallProgress, InstallStage,
};
use crate::platform::download::download_with_progress;

pub async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let platform_ext = format!("darwin-{arch}.tar.gz");
    let urls = build_node_urls(nodejs_dist_base, &platform_ext)?;

    let node_dir = clawenv_node_dir();
    tokio::fs::create_dir_all(&node_dir).await?;
    let tar_path = node_dir.parent().unwrap_or(&node_dir).join("node.tar.gz");

    let bytes = download_with_progress(
        &urls, None, tx,
        InstallStage::EnsurePrerequisites,
        14, 18,
        &format!("Node.js darwin-{arch}"),
    ).await?;
    tokio::fs::write(&tar_path, &bytes).await?;

    send(tx, "Extracting Node.js...", 18, InstallStage::EnsurePrerequisites).await;

    // Extract tar.gz, strip the top-level directory
    let status = tokio::process::Command::new("tar")
        .args(["xzf", &tar_path.to_string_lossy(), "--strip-components=1", "-C", &node_dir.to_string_lossy()])
        .status()
        .await?;

    tokio::fs::remove_file(&tar_path).await.ok();

    if !status.success() {
        anyhow::bail!("Failed to extract Node.js");
    }

    send(tx, "Node.js installed to ~/.clawenv/node", 22, InstallStage::EnsurePrerequisites).await;
    ensure_node_in_path();

    Ok(())
}
