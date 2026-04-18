//! Unified exec helper for all sandbox backends.
//!
//! Two modes:
//! - `exec()`: For short commands (echo, which, cat, etc.). Has pipe-read timeout.
//! - `exec_with_progress()`: For long commands (apk add, npm install, etc.).
//!   Streams output line-by-line with idle timeout — if no output arrives for
//!   IDLE_TIMEOUT_SECS, the process is killed and an error is returned.

use anyhow::Result;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

/// Pipe-read timeout for `exec()` (short commands only).
const READ_TIMEOUT_SECS: u64 = 120;

/// Idle timeout for `exec_with_progress()` — if no stdout/stderr line arrives
/// for this long, the process is considered stalled and is killed.
/// 20 minutes covers slow networks, npm postinstall hangs after optional-dep
/// failures, and silent npm phases (lockfile write, bin-linking) while still
/// catching truly hung processes. Background-script runs also emit a 30-second
/// heartbeat, so this ceiling is mostly a safety net.
const IDLE_TIMEOUT_SECS: u64 = 1200;

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

/// Execute a long-running command with streaming progress and idle timeout.
///
/// Pipes stdout+stderr lines to a channel as they arrive.
/// **Idle timeout**: if no output line arrives for 10 minutes, the process is
/// killed and an error is returned. Any output resets the timer — so slow but
/// active downloads (producing progress output) will never be timed out.
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

    // Merge stdout+stderr into a single channel for unified idle detection.
    // The merged channel feeds both the caller's tx and output collection.
    let (line_tx, mut line_rx) = mpsc::channel::<(String, bool)>(128); // (line, is_stdout)

    // Stderr reader
    let line_tx2 = line_tx.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(se) = stderr {
            let mut reader = BufReader::new(se).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line_tx2.send((line, false)).await.is_err() { break; }
            }
        }
    });

    // Stdout reader
    let line_tx3 = line_tx.clone();
    let stdout_task = tokio::spawn(async move {
        if let Some(so) = stdout {
            let mut reader = BufReader::new(so).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line_tx3.send((line, true)).await.is_err() { break; }
            }
        }
    });

    // Drop our copy so channel closes when both readers finish
    drop(line_tx);

    // Consume lines with idle timeout
    let mut output = String::new();
    let idle_timeout = Duration::from_secs(IDLE_TIMEOUT_SECS);

    loop {
        match tokio::time::timeout(idle_timeout, line_rx.recv()).await {
            Ok(Some((line, is_stdout))) => {
                // Activity received — forward to caller and collect stdout
                let _ = tx.send(line.clone()).await;
                if is_stdout {
                    output.push_str(&line);
                    output.push('\n');
                }
            }
            Ok(None) => {
                // Channel closed — both readers finished (process exited or pipes closed)
                break;
            }
            Err(_) => {
                // Idle timeout — no output for IDLE_TIMEOUT_SECS
                tracing::error!(
                    "exec_with_progress idle timeout: no output for {}s, killing process",
                    IDLE_TIMEOUT_SECS
                );
                child.kill().await.ok();
                anyhow::bail!(
                    "Process stalled — no output for {} minutes. The operation may be stuck. \
                     Check network connectivity and try again.",
                    IDLE_TIMEOUT_SECS / 60
                );
            }
        }
    }

    // Wait for process exit
    let status = child.wait().await;

    // Clean up reader tasks
    stderr_task.abort();
    stdout_task.abort();

    let code = match status {
        Ok(s) => s.code().unwrap_or(-1),
        Err(e) => { tracing::error!("exec wait failed: {e}"); -1 }
    };

    Ok((output, code))
}

// These tests invoke Unix shell primitives (`echo`, `sh -c …`) directly as
// standalone binaries — on Windows `echo` is a cmd builtin (not on PATH as an
// executable) and `sh` doesn't exist outside of MSYS/Git-Bash. Gate the whole
// module to unix; a separate cross-platform smoke test lives in core's
// top-level tests module (`test_exec_echo_on_current_platform`).
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exec_success() {
        let (stdout, _stderr, code) = exec("echo", &["hello"]).await.unwrap();
        assert_eq!(code, 0);
        assert!(stdout.trim().contains("hello"));
    }

    #[tokio::test]
    async fn test_exec_exit_code() {
        let (_stdout, _stderr, code) = exec("sh", &["-c", "exit 42"]).await.unwrap();
        assert_eq!(code, 42);
    }

    #[tokio::test]
    async fn test_exec_stderr() {
        let (_stdout, stderr, code) = exec("sh", &["-c", "echo err >&2"]).await.unwrap();
        assert_eq!(code, 0);
        assert!(stderr.contains("err"));
    }

    #[tokio::test]
    async fn test_exec_with_progress_success() {
        let (tx, mut rx) = mpsc::channel(32);
        let (output, code) = exec_with_progress("echo", &["progress-test"], &tx).await.unwrap();
        assert_eq!(code, 0);
        assert!(output.contains("progress-test"));

        // Channel should have received the line
        let line = rx.try_recv();
        assert!(line.is_ok());
        assert!(line.unwrap().contains("progress-test"));
    }

    #[tokio::test]
    async fn test_exec_with_progress_multiline() {
        let (tx, mut rx) = mpsc::channel(32);
        let (output, code) = exec_with_progress(
            "sh",
            &["-c", "echo line1; echo line2; echo line3"],
            &tx,
        ).await.unwrap();
        assert_eq!(code, 0);
        assert!(output.contains("line1"));
        assert!(output.contains("line3"));

        // All lines forwarded
        let mut lines = Vec::new();
        while let Ok(l) = rx.try_recv() {
            lines.push(l);
        }
        assert!(lines.len() >= 3);
    }

    #[tokio::test]
    async fn test_exec_with_progress_exit_code() {
        let (tx, _rx) = mpsc::channel(32);
        let (_output, code) = exec_with_progress("sh", &["-c", "echo ok; exit 7"], &tx).await.unwrap();
        assert_eq!(code, 7);
    }
}
