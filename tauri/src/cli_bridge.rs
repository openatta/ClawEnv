//! Bridge between Tauri GUI and the CLI binary.
//!
//! Instead of calling clawenv-core directly, the GUI spawns `clawenv-cli --json <command>`
//! and parses the JSON-lines output. This keeps the CLI as the single source of truth for
//! all business logic, and the GUI as a thin presentation shell.
//!
//! **Idle timeout**: both `run_cli` and `run_cli_streaming` use activity-based timeouts.
//! Any JSON line received from CLI resets the timer. If no output arrives for
//! IDLE_TIMEOUT_SECS, the child process is killed and an error is returned.
//! This prevents the GUI from hanging forever when CLI stalls, while allowing
//! legitimately slow operations (large downloads, npm install) that produce
//! periodic progress output.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Idle timeout — if no JSON line arrives from CLI for this long,
/// the process is considered stalled. 10 minutes matches core's idle detection.
const IDLE_TIMEOUT_SECS: u64 = 600;

/// A parsed CLI event (mirrors cli/src/output.rs CliEvent).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliEvent {
    Progress { stage: String, percent: u8, message: String },
    Info { message: String },
    Complete { message: String },
    Error { message: String },
    Data { data: Value },
}

/// Resolve the CLI binary path.
///
/// Search order:
/// 1. Same directory as the running Tauri exe (production bundle)
/// 2. Workspace target/debug or target/release (dev mode)
/// 3. System PATH (fallback)
fn cli_binary_path() -> String {
    #[cfg(target_os = "windows")]
    let cli_name = "clawenv-cli.exe";
    #[cfg(not(target_os = "windows"))]
    let cli_name = "clawenv-cli";

    // 1. Same directory as current exe (production)
    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(std::path::Path::new("."));
        let bundled = dir.join(cli_name);
        if bundled.exists() {
            return bundled.to_string_lossy().to_string();
        }

        // 2. Dev mode: walk up from target/debug/clawenv-tauri to target/debug/clawenv-cli
        // Tauri exe is at: <workspace>/target/debug/clawenv-tauri
        // CLI binary is at: <workspace>/target/debug/clawenv-cli
        // They share the same target directory.
        if dir.ends_with("debug") || dir.ends_with("release") {
            let sibling = dir.join(cli_name);
            if sibling.exists() {
                return sibling.to_string_lossy().to_string();
            }
        }
    }

    // 3. Fallback: PATH
    cli_name.into()
}

fn new_cli_command(binary: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(binary);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Run a CLI command and return the final result.
/// Uses idle timeout — if no output for IDLE_TIMEOUT_SECS, kills and returns error.
pub async fn run_cli(args: &[&str]) -> Result<Value> {
    let binary = cli_binary_path();
    let mut cmd = new_cli_command(&binary);
    cmd.arg("--json").args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn()
        .with_context(|| format!("Failed to run CLI: {} --json {}", binary, args.join(" ")))?;

    let stdout = child.stdout.take().context("No stdout")?;
    let mut reader = BufReader::new(stdout).lines();

    let idle_timeout = Duration::from_secs(IDLE_TIMEOUT_SECS);
    let mut last_data: Option<Value> = None;
    let mut last_complete: Option<String> = None;

    // Read lines with idle timeout
    loop {
        match tokio::time::timeout(idle_timeout, reader.next_line()).await {
            Ok(Ok(Some(line))) => {
                // Activity — parse event
                if let Ok(event) = serde_json::from_str::<CliEvent>(&line) {
                    match event {
                        CliEvent::Data { data } => last_data = Some(data),
                        CliEvent::Complete { message } => last_complete = Some(message),
                        CliEvent::Error { message } => anyhow::bail!("{}", message),
                        _ => {}
                    }
                }
            }
            Ok(Ok(None)) => break,   // EOF — process finished
            Ok(Err(_)) => break,      // IO error — process likely dead
            Err(_) => {
                // Idle timeout
                child.kill().await.ok();
                anyhow::bail!(
                    "CLI command timed out — no output for {} minutes. \
                     The operation may be stuck. Check network and try again.",
                    IDLE_TIMEOUT_SECS / 60
                );
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("CLI failed (exit {})", status);
    }

    if let Some(data) = last_data {
        Ok(data)
    } else if let Some(msg) = last_complete {
        Ok(Value::String(msg))
    } else {
        Ok(Value::Null)
    }
}

/// Run a CLI command with streaming events forwarded via a channel.
/// Used for long-running operations like install and upgrade.
///
/// **Idle timeout**: if no JSON line arrives for IDLE_TIMEOUT_SECS, the child
/// process is killed and an error is returned. Any output resets the timer.
pub async fn run_cli_streaming(
    args: &[&str],
    tx: mpsc::Sender<CliEvent>,
) -> Result<Value> {
    let binary = cli_binary_path();
    let mut cmd = new_cli_command(&binary);
    cmd.arg("--json").args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn()
        .with_context(|| format!("Failed to spawn CLI: {} --json {}", binary, args.join(" ")))?;

    let stdout = child.stdout.take().context("No stdout")?;
    let mut reader = BufReader::new(stdout).lines();

    let idle_timeout = Duration::from_secs(IDLE_TIMEOUT_SECS);
    let mut last_data: Option<Value> = None;
    let mut last_complete: Option<String> = None;
    let mut last_error: Option<String> = None;

    loop {
        match tokio::time::timeout(idle_timeout, reader.next_line()).await {
            Ok(Ok(Some(line))) => {
                // Activity received — parse and forward
                if let Ok(event) = serde_json::from_str::<CliEvent>(&line) {
                    match &event {
                        CliEvent::Data { data } => last_data = Some(data.clone()),
                        CliEvent::Complete { message } => last_complete = Some(message.clone()),
                        CliEvent::Error { message } => last_error = Some(message.clone()),
                        _ => {}
                    }
                    let _ = tx.send(event).await;
                }
            }
            Ok(Ok(None)) => break,   // EOF
            Ok(Err(_)) => break,      // IO error
            Err(_) => {
                // Idle timeout — kill child and report error
                child.kill().await.ok();
                let err_msg = format!(
                    "Operation timed out — no progress for {} minutes. \
                     The operation may be stuck. Check network and try again.",
                    IDLE_TIMEOUT_SECS / 60
                );
                let _ = tx.send(CliEvent::Error { message: err_msg.clone() }).await;
                anyhow::bail!("{}", err_msg);
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        if let Some(err) = last_error {
            anyhow::bail!("{}", err);
        }
        anyhow::bail!("CLI exited with status {}", status);
    }

    if let Some(data) = last_data {
        Ok(data)
    } else if let Some(msg) = last_complete {
        Ok(Value::String(msg))
    } else {
        Ok(Value::Null)
    }
}
