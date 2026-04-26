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
/// v0.3.0: the `urls` slice is effectively always length 1 (upstream
/// only — fallback tiers removed). We still accept a slice for the
/// benefit of legacy callers (and in case a user override later gets
/// plumbed in as a pre-pended alt URL), but the body takes the first
/// entry and treats its failure as a chance for a single retry with
/// exponential backoff — *not* as a cue to cycle through a tier list.
/// The old `last_err + continue` state machine was dead code once the
/// URL list collapsed.
///
/// - `urls` — (full_url, filename) pairs. First entry is used; rest are
///   ignored. Filename is only used in log lines — caller writes bytes.
/// - `expected_sha256` — hex digest. `None` = trust TLS, don't verify.
///   Any mismatch fails the download (no silent wrong-bytes return).
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
    use std::time::Duration;

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
    // How many total attempts (original + retries). 2 covers one
    // transient-hiccup retry without masking a truly broken network
    // (at which point v0.3.0 contract says: surface the failure).
    const MAX_ATTEMPTS: u32 = 2;

    let (url, _filename) = urls.first()
        .ok_or_else(|| anyhow::anyhow!("download {label}: no URL provided"))?;

    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()?;

    let mut last_err: Option<String> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match try_download_single(
            &client, url, expected_sha256, tx, stage.clone(),
            start_percent, end_percent, label,
        ).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                let msg = e.to_string();
                tracing::warn!(target: "clawenv::proxy",
                    "download {label} attempt {attempt}/{MAX_ATTEMPTS}: {msg}");
                last_err = Some(msg);
                if attempt < MAX_ATTEMPTS {
                    // Short backoff between retries — enough to let a
                    // transient blip clear, not so long the user thinks
                    // we hung.
                    tokio::time::sleep(Duration::from_secs(attempt as u64 * 2)).await;
                }
            }
        }
    }

    anyhow::bail!(
        "{label} download failed after {MAX_ATTEMPTS} attempts. Last error: {}",
        last_err.as_deref().unwrap_or("(unknown)")
    )
}

/// Single-attempt streaming download + sha256 verify. Factored out of
/// `download_with_progress` so the outer retry loop is trivial.
#[allow(clippy::too_many_arguments)]
async fn try_download_single(
    client: &reqwest::Client,
    url: &str,
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

    const CHUNK_STALL: Duration = Duration::from_secs(60);
    const PROGRESS_BYTES: u64 = 1024 * 1024;
    const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
    // Throughput floor: if the first 256 KB hasn't arrived within 30s
    // we abort. Pure CHUNK_STALL doesn't catch "trickle mode" where
    // the connection delivers 1 byte every 29s — chunk timer resets
    // per byte and the URL never fails.
    const MIN_BYTES_BY_DEADLINE: u64 = 256 * 1024;
    const MIN_THROUGHPUT_DEADLINE: Duration = Duration::from_secs(30);

    let start = start_percent as u64;
    let span = (end_percent.saturating_sub(start_percent)) as u64;

    tracing::info!(target: "clawenv::proxy", "download {label}: {url}");
    send(tx, &format!("Connecting to {label}..."), start_percent, stage.clone()).await;

    let mut resp = match client.get(url).send().await {
        Err(e) => anyhow::bail!("{url}: {e}"),
        Ok(r) if !r.status().is_success() => anyhow::bail!("{url}: HTTP {}", r.status()),
        Ok(r) => r,
    };

    let total = resp.content_length();
    let mut hasher = Sha256::new();
    let cap = total.map(|t| t as usize).unwrap_or(0);
    let mut buf: Vec<u8> = Vec::with_capacity(cap);
    let t0 = Instant::now();
    let mut last_emit_bytes: u64 = 0;
    let mut last_emit_at = t0;

    loop {
        match timeout(CHUNK_STALL, resp.chunk()).await {
            Err(_) => anyhow::bail!(
                "{url}: stalled — no data for {}s", CHUNK_STALL.as_secs()
            ),
            Ok(Err(e)) => anyhow::bail!("{url}: body read: {e}"),
            Ok(Ok(None)) => break, // EOF
            Ok(Ok(Some(chunk))) => {
                hasher.update(&chunk);
                buf.extend_from_slice(&chunk);

                let downloaded = buf.len() as u64;

                if t0.elapsed() >= MIN_THROUGHPUT_DEADLINE
                    && downloaded < MIN_BYTES_BY_DEADLINE
                {
                    anyhow::bail!(
                        "{url}: throughput floor — only {} bytes in {}s",
                        downloaded, MIN_THROUGHPUT_DEADLINE.as_secs()
                    );
                }

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

    if let Some(expected) = expected_sha256 {
        let hex = hex::encode(hasher.finalize());
        if hex != expected {
            anyhow::bail!("{url}: checksum mismatch (got {hex}, want {expected})");
        }
    }

    let final_msg = format_download_msg(label, buf.len() as u64, total, t0.elapsed());
    send(tx, &final_msg, end_percent, stage.clone()).await;
    Ok(buf)
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
    use std::time::{Duration, Instant};
    use tokio::time::timeout;

    const CHUNK_STALL: Duration = Duration::from_secs(30);
    // Same throughput floor as download_with_progress — see that fn for
    // rationale (GFW trickle-mode bypass).
    const MIN_BYTES_BY_DEADLINE: u64 = 256 * 1024;
    const MIN_THROUGHPUT_DEADLINE: Duration = Duration::from_secs(30);

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
        let mut failed: Option<String> = None;
        let t0 = Instant::now();
        loop {
            match timeout(CHUNK_STALL, resp.chunk()).await {
                Err(_) | Ok(Err(_)) => { failed = Some(format!("{url}: stalled")); break; }
                Ok(Ok(None)) => break,
                Ok(Ok(Some(c))) => {
                    hasher.update(&c);
                    buf.extend_from_slice(&c);
                    if t0.elapsed() >= MIN_THROUGHPUT_DEADLINE
                        && (buf.len() as u64) < MIN_BYTES_BY_DEADLINE
                    {
                        failed = Some(format!(
                            "{url}: throughput floor — only {} bytes in {}s",
                            buf.len(), MIN_THROUGHPUT_DEADLINE.as_secs()
                        ));
                        break;
                    }
                }
            }
        }
        if let Some(e) = failed { last_err = Some(e); continue; }
        if let Some(expected) = expected_sha256 {
            let hex = hex::encode(hasher.finalize());
            if hex != expected { last_err = Some(format!("{url}: sha mismatch")); continue; }
        }
        return Ok(buf);
    }
    anyhow::bail!("All URLs failed: {}", last_err.as_deref().unwrap_or("?"))
}
