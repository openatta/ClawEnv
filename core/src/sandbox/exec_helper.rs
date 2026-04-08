//! Unified exec helper for all sandbox backends.
//!
//! Two modes:
//! - `exec()`: For short commands (echo, which, cat, etc.). Has pipe-read timeout.
//! - `exec_with_progress()`: For long commands (apk add, npm install, etc.).
//!   Streams output line-by-line with NO timeout — the process runs until it
//!   finishes naturally. Caller shows progress via heartbeat.

use anyhow::Result;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

/// Pipe-read timeout for `exec()` (short commands only).
const READ_TIMEOUT_SECS: u64 = 120;

/// Read from an async reader with a timeout.
async fn read_with_timeout(mut reader: impl AsyncReadExt + Unpin, secs: u64) -> String {
    let mut buf = Vec::new();
    match tokio::time::timeout(Duration::from_secs(secs), reader.read_to_end(&mut buf)).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) => {}
        Err(_) => {
            tracing::warn!("exec pipe read timed out after {secs}s ({} bytes read)", buf.len());
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Execute a short command, returning (stdout, stderr, exit_code).
/// Has a 120s pipe-read timeout — use `exec_with_progress` for long operations.
pub async fn exec(program: &str, args: &[&str]) -> Result<(String, String, i32)> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout_h = child.stdout.take()
        .ok_or_else(|| anyhow::anyhow!("stdout pipe not available"))?;
    let stderr_h = child.stderr.take()
        .ok_or_else(|| anyhow::anyhow!("stderr pipe not available"))?;

    let (status_result, stdout, stderr) = tokio::join!(
        child.wait(),
        read_with_timeout(stdout_h, READ_TIMEOUT_SECS),
        read_with_timeout(stderr_h, READ_TIMEOUT_SECS),
    );

    let code = match status_result {
        Ok(s) => s.code().unwrap_or(-1),
        Err(e) => {
            tracing::error!("exec wait failed: {e}");
            -1
        }
    };

    Ok((stdout, stderr, code))
}

/// Execute a long-running command with streaming progress.
///
/// Pipes stdout+stderr lines to a channel as they arrive.
/// **No timeout** — the process runs until it finishes naturally.
/// Caller is responsible for showing heartbeat/progress to the user.
pub async fn exec_with_progress(
    program: &str,
    args: &[&str],
    tx: &mpsc::Sender<String>,
) -> Result<(String, i32)> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Stream stderr lines to channel
    let tx2 = tx.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(se) = stderr {
            let mut reader = BufReader::new(se).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx2.send(line).await;
            }
        }
    });

    // Stream stdout lines to channel + collect full output
    let tx3 = tx.clone();
    let stdout_task = tokio::spawn(async move {
        let mut output = String::new();
        if let Some(so) = stdout {
            let mut reader = BufReader::new(so).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx3.send(line.clone()).await;
                output.push_str(&line);
                output.push('\n');
            }
        }
        output
    });

    // Wait for process to finish — NO timeout
    let status = child.wait().await;

    // Give pipes a moment to flush remaining data
    tokio::time::sleep(Duration::from_millis(500)).await;
    stderr_task.abort();
    let output = stdout_task.await.unwrap_or_default();

    let code = match status {
        Ok(s) => s.code().unwrap_or(-1),
        Err(e) => { tracing::error!("exec wait failed: {e}"); -1 }
    };

    Ok((output, code))
}
