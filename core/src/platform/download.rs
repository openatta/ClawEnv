//! Streaming HTTP download with progress + stall detection + mirror
//! fallback + optional sha256 verification.
//!
//! This is the single downloader every installer uses:
//! - Git (dugite / MinGit) in `install_native/mod.rs`
//! - Node.js in `install_native/{macos,windows,linux}.rs`
//! - Lima in `sandbox/lima.rs`
//! - WSL2 Alpine rootfs in `sandbox/wsl.rs`
//! - Podman / Alpine in `sandbox/podman.rs`
//! - `update/checker.rs` (short-timeout variant)
//!
//! Why these choices:
//! - **Connect timeout 15s**: kill dead mirrors fast so fallback proceeds.
//! - **Per-chunk stall 60s**: don't confuse "slow" with "stuck". A 50MB
//!   download on a 200 KB/s link is fine; a connection that delivered
//!   nothing for a minute is hung.
//! - **Progress throttle 1 MiB or 500ms**: each emit also resets the
//!   cli_bridge idle watcher in Tauri, so a long-but-progressing download
//!   never triggers the 10-minute kill.

use anyhow::Result;
use tokio::sync::mpsc;

pub use super::super::manager::install::{InstallProgress, InstallStage};

/// Downloader signature used by every installer. Progress events flow
/// through `tx` and drive the UI progress bar as well as keeping the
/// CLI-bridge idle watcher alive.
///
/// - `urls` — (full_url, filename) pairs. First success returns. Filename
///   is only used in log lines — the caller writes the resulting bytes
///   wherever it wants.
/// - `expected_sha256` — hex digest. `None` = trust TLS, don't verify.
///   Any mismatch bails (moves to next URL, doesn't return bad bytes).
/// - `tx` / `stage` / `start_percent` / `end_percent` — progress mapping.
/// - `label` — user-facing description ("portable Git (macOS-arm64)").
pub async fn download_with_progress(
    urls: &[(String, String)],
    expected_sha256: Option<&str>,
    tx: &mpsc::Sender<InstallProgress>,
    stage: InstallStage,
    start_percent: u8,
    end_percent: u8,
    label: &str,
) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};
    use std::time::{Duration, Instant};
    use tokio::time::timeout;

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
    const CHUNK_STALL: Duration = Duration::from_secs(60);
    const PROGRESS_BYTES: u64 = 1024 * 1024;
    const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);

    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()?;

    let mut last_err: Option<String> = None;
    let start = start_percent as u64;
    let span = (end_percent.saturating_sub(start_percent)) as u64;

    for (url, _filename) in urls {
        tracing::info!(target: "clawenv::proxy", "download {label}: trying {url}");
        send(tx, &format!("Connecting to {label}..."), start_percent, stage.clone()).await;

        let mut resp = match client.get(url).send().await {
            Err(e) => { last_err = Some(format!("{url}: {e}")); continue; }
            Ok(r) if !r.status().is_success() => {
                last_err = Some(format!("{url}: HTTP {}", r.status())); continue;
            }
            Ok(r) => r,
        };

        let total = resp.content_length();
        let mut hasher = Sha256::new();
        let cap = total.map(|t| t as usize).unwrap_or(0);
        let mut buf: Vec<u8> = Vec::with_capacity(cap);
        let t0 = Instant::now();
        let mut last_emit_bytes: u64 = 0;
        let mut last_emit_at = t0;
        let mut url_failed: Option<String> = None;

        loop {
            match timeout(CHUNK_STALL, resp.chunk()).await {
                Err(_) => {
                    url_failed = Some(format!(
                        "{url}: stalled — no data for {}s",
                        CHUNK_STALL.as_secs()
                    ));
                    break;
                }
                Ok(Err(e)) => {
                    url_failed = Some(format!("{url}: body read: {e}"));
                    break;
                }
                Ok(Ok(None)) => break, // EOF
                Ok(Ok(Some(chunk))) => {
                    hasher.update(&chunk);
                    buf.extend_from_slice(&chunk);

                    let downloaded = buf.len() as u64;
                    let since_bytes = downloaded - last_emit_bytes;
                    if since_bytes >= PROGRESS_BYTES || last_emit_at.elapsed() >= PROGRESS_INTERVAL {
                        let pct = if let Some(t) = total {
                            if t > 0 { (start + span * downloaded / t).min(end_percent as u64) }
                            else { start }
                        } else { start } as u8;
                        send(tx, &format_download_msg(label, downloaded, total, t0.elapsed()),
                             pct, stage.clone()).await;
                        last_emit_bytes = downloaded;
                        last_emit_at = Instant::now();
                    }
                }
            }
        }

        if let Some(err) = url_failed {
            tracing::warn!(target: "clawenv::proxy", "{err}");
            last_err = Some(err);
            continue;
        }

        if let Some(expected) = expected_sha256 {
            let hex = hex::encode(hasher.finalize());
            if hex != expected {
                last_err = Some(format!("{url}: checksum mismatch (got {hex})"));
                continue;
            }
        }

        let final_msg = format_download_msg(label, buf.len() as u64, total, t0.elapsed());
        send(tx, &final_msg, end_percent, stage.clone()).await;
        return Ok(buf);
    }

    anyhow::bail!(
        "All {label} download URLs failed. Last error: {}",
        last_err.as_deref().unwrap_or("(no URLs tried)")
    )
}

fn format_download_msg(label: &str, downloaded: u64, total: Option<u64>, elapsed: std::time::Duration) -> String {
    let mb = |b: u64| b as f64 / 1024.0 / 1024.0;
    let secs = elapsed.as_secs_f64().max(0.001);
    let rate = mb(downloaded) / secs;
    match total {
        Some(t) => format!("Downloading {label}: {:.1} / {:.1} MB ({:.1} MB/s)", mb(downloaded), mb(t), rate),
        None => format!("Downloading {label}: {:.1} MB ({:.1} MB/s)", mb(downloaded), rate),
    }
}

async fn send(tx: &mpsc::Sender<InstallProgress>, message: &str, percent: u8, stage: InstallStage) {
    let _ = tx.send(InstallProgress {
        message: message.to_string(),
        percent,
        stage,
    }).await;
}

/// Headless variant for callers that don't have an `InstallProgress`
/// channel (e.g. `update/checker.rs`). Shares the same stall detection
/// + mirror fallback + sha256 semantics, silently. Uses a shorter
/// connect timeout — update checks shouldn't wait 15s per failing
/// mirror.
pub async fn download_silent(
    urls: &[(String, String)],
    expected_sha256: Option<&str>,
    connect_timeout_secs: u64,
) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};
    use std::time::Duration;
    use tokio::time::timeout;

    const CHUNK_STALL: Duration = Duration::from_secs(30);

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(connect_timeout_secs))
        .build()?;
    let mut last_err: Option<String> = None;
    for (url, _) in urls {
        let mut resp = match client.get(url).send().await {
            Err(e) => { last_err = Some(format!("{url}: {e}")); continue; }
            Ok(r) if !r.status().is_success() => {
                last_err = Some(format!("{url}: HTTP {}", r.status())); continue;
            }
            Ok(r) => r,
        };
        let mut buf = Vec::new();
        let mut hasher = Sha256::new();
        let mut failed = false;
        loop {
            match timeout(CHUNK_STALL, resp.chunk()).await {
                Err(_) | Ok(Err(_)) => { failed = true; break; }
                Ok(Ok(None)) => break,
                Ok(Ok(Some(c))) => { hasher.update(&c); buf.extend_from_slice(&c); }
            }
        }
        if failed { last_err = Some(format!("{url}: stalled")); continue; }
        if let Some(expected) = expected_sha256 {
            let hex = hex::encode(hasher.finalize());
            if hex != expected { last_err = Some(format!("{url}: sha mismatch")); continue; }
        }
        return Ok(buf);
    }
    anyhow::bail!("All URLs failed: {}", last_err.as_deref().unwrap_or("?"))
}
