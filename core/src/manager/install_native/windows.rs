//! Windows-specific Node.js installation for Native mode.
//!
//! Downloads .zip from nodejs.org, extracts to ~/.clawenv/node/ using PowerShell.
//! No admin privileges needed — fully self-contained in user directory.

use anyhow::Result;
use tokio::sync::mpsc;

use super::{InstallProgress, InstallStage, send, clawenv_node_dir, ensure_node_in_path};

pub async fn install_nodejs(tx: &mpsc::Sender<InstallProgress>, nodejs_dist_base: &str) -> Result<()> {
    send(tx, "Downloading Node.js for Windows...", 14, InstallStage::EnsurePrerequisites).await;

    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = "v22.16.0";
    let url = format!("{nodejs_dist_base}/{version}/node-{version}-win-{arch}.zip");

    // Install to ~/.clawenv/node/ (no admin needed, self-contained)
    let node_dir = clawenv_node_dir();
    tokio::fs::create_dir_all(&node_dir).await?;

    let zip_path = node_dir.parent().unwrap_or(&node_dir).join("node.zip");

    // Download using curl.exe (built into Windows 11)
    let status = crate::platform::process::silent_cmd("curl.exe")
        .args(["-fSL", "-o", &zip_path.to_string_lossy(), &url])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to download Node.js from {url}");
    }

    send(tx, "Extracting Node.js...", 18, InstallStage::EnsurePrerequisites).await;

    // Extract zip using PowerShell (no admin needed)
    // 1. Expand-Archive into node dir
    // 2. Move contents from nested node-vX.Y.Z-win-arch/ up to node dir
    let extract_cmd = format!(
        "Expand-Archive -Path '{}' -DestinationPath '{}' -Force; \
         $d = Get-ChildItem '{}' -Directory | Select-Object -First 1; \
         if ($d) {{ Get-ChildItem $d.FullName | Move-Item -Destination '{}' -Force; Remove-Item $d.FullName -Force }}",
        zip_path.to_string_lossy(),
        node_dir.to_string_lossy(),
        node_dir.to_string_lossy(),
        node_dir.to_string_lossy(),
    );
    let status = crate::platform::process::silent_cmd("powershell")
        .args(["-WindowStyle", "Hidden", "-Command", &extract_cmd])
        .status()
        .await?;

    // Cleanup zip
    tokio::fs::remove_file(&zip_path).await.ok();

    if !status.success() {
        anyhow::bail!("Failed to extract Node.js");
    }

    send(tx, "Node.js installed to ~/.clawenv/node", 22, InstallStage::EnsurePrerequisites).await;

    // Add to PATH for this process
    ensure_node_in_path();

    Ok(())
}
