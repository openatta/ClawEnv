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

    let node_dir = clawenv_node_dir();
    let parent = node_dir.parent().unwrap_or(&node_dir).to_path_buf();
    let zip_path = parent.join("node.zip");
    let tmp_dir = parent.join("node-tmp");

    // Download using curl.exe (built into Windows 10+)
    let status = crate::platform::process::silent_cmd("curl.exe")
        .args(["-fSL", "-o", &zip_path.to_string_lossy(), &url])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to download Node.js from {url}");
    }

    send(tx, "Extracting Node.js (this may take a moment)...", 18, InstallStage::EnsurePrerequisites).await;

    // Clean previous installs
    let cleanup_cmd = format!(
        "Remove-Item -Recurse -Force '{}' -ErrorAction SilentlyContinue; \
         Remove-Item -Recurse -Force '{}' -ErrorAction SilentlyContinue",
        node_dir.to_string_lossy(),
        tmp_dir.to_string_lossy(),
    );
    crate::platform::process::silent_cmd("powershell")
        .args(["-Command", &cleanup_cmd])
        .status().await.ok();

    // Extract to temp dir
    let extract_cmd = format!(
        "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
        zip_path.to_string_lossy(),
        tmp_dir.to_string_lossy(),
    );
    let status = crate::platform::process::silent_cmd("powershell")
        .args(["-Command", &extract_cmd])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to extract Node.js zip");
    }

    // Move the nested directory (e.g. node-v22.16.0-win-arm64/) to final location
    // The zip contains exactly one top-level directory
    let move_cmd = format!(
        "$src = Get-ChildItem '{}' -Directory | Select-Object -First 1; \
         if ($src) {{ \
           Move-Item -Path $src.FullName -Destination '{}' -Force \
         }} else {{ \
           Move-Item -Path '{}' -Destination '{}' -Force \
         }}",
        tmp_dir.to_string_lossy(),
        node_dir.to_string_lossy(),
        tmp_dir.to_string_lossy(),
        node_dir.to_string_lossy(),
    );
    let status = crate::platform::process::silent_cmd("powershell")
        .args(["-Command", &move_cmd])
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("Failed to move Node.js files");
    }

    // Cleanup
    tokio::fs::remove_file(&zip_path).await.ok();
    tokio::fs::remove_dir_all(&tmp_dir).await.ok();

    // Verify npm.cmd exists
    let npm_cmd = node_dir.join("npm.cmd");
    if !npm_cmd.exists() {
        anyhow::bail!("Node.js extraction incomplete: npm.cmd not found in {}", node_dir.display());
    }

    send(tx, "Node.js installed to ~/.clawenv/node", 22, InstallStage::EnsurePrerequisites).await;
    ensure_node_in_path();

    Ok(())
}
