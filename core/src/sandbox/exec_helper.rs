//! Plan C: Unified exec helper for all sandbox backends.
//!
//! Uses `tokio::join!(wait, read_stdout, read_stderr)` with timeout
//! on pipe reads. Works on all platforms — Lima, WSL2, Podman, Native.

use anyhow::Result;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

const READ_TIMEOUT_SECS: u64 = 30;

/// Read from an async reader with a timeout.
/// Returns whatever was read before timeout (may be partial).
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

/// Execute a command, returning (stdout, stderr, exit_code).
/// Uses Plan C: spawn with pipes, join!(wait, read, read) with timeout.
pub async fn exec(program: &str, args: &[&str]) -> Result<(String, String, i32)> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout_h = child.stdout.take().unwrap();
    let stderr_h = child.stderr.take().unwrap();

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

/// Execute with streaming progress — pipes stdout+stderr lines to a channel.
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

    // Stream stderr to channel
    let tx2 = tx.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(se) = stderr {
            let mut reader = BufReader::new(se).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx2.send(line).await;
            }
        }
    });

    // Stream stdout to channel + collect
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

    // Wait for process with global timeout
    let wait_result = tokio::time::timeout(
        Duration::from_secs(300), // 5 min max
        child.wait(),
    ).await;

    // Give pipes a moment to flush
    tokio::time::sleep(Duration::from_millis(300)).await;
    stderr_task.abort();
    let output = stdout_task.await.unwrap_or_default();

    let code = match wait_result {
        Ok(Ok(s)) => s.code().unwrap_or(-1),
        Ok(Err(e)) => { tracing::error!("exec wait failed: {e}"); -1 }
        Err(_) => { tracing::error!("exec timed out (5 min)"); -1 }
    };

    Ok((output, code))
}
