//! Bridge between Tauri GUI and the CLI binary.
//!
//! Spawns `clawcli --json <command>` and parses JSON-lines output.
//! CLI is the single source of truth; GUI is a thin presentation shell.
//!
//! **Idle timeout**: activity-based — any stdout/stderr line resets the timer.
//! **stderr**: captured and logged via tracing (CLI tracing logs, error details).
//! **Error codes**: preserved from CLI Error events and forwarded to frontend.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};

/// Idle timeout — 10 minutes matches core's idle detection.
const IDLE_TIMEOUT_SECS: u64 = 600;

/// Per-instance mutex to prevent duplicate concurrent operations (e.g., double-click start).
/// Key is the instance name extracted from CLI args; value is a mutex guarding that instance.
static INSTANCE_LOCKS: LazyLock<Mutex<HashMap<String, std::sync::Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Extract the instance name from CLI args for locking purposes.
/// Matches patterns like: ["start", "--name", "default"] or ["install", "--name", "my-instance"]
fn extract_instance_name(args: &[&str]) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if (*arg == "--name" || *arg == "-n") && i + 1 < args.len() {
            return Some(args[i + 1].to_string());
        }
    }
    // For subcommands that take name as first positional arg after verb
    if args.len() >= 2 {
        let verb = args[0];
        if ["start", "stop", "restart", "uninstall", "upgrade", "export"].contains(&verb) {
            // Check if second arg looks like a name (not a flag)
            if !args[1].starts_with('-') {
                return Some(args[1].to_string());
            }
        }
    }
    None
}

/// Acquire a per-instance lock. Returns the guard that must be held for the operation's duration.
async fn acquire_instance_lock(args: &[&str]) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    let name = extract_instance_name(args)?;
    let lock = {
        let mut locks = INSTANCE_LOCKS.lock().await;
        locks.entry(name).or_insert_with(|| std::sync::Arc::new(Mutex::new(()))).clone()
    };
    Some(lock.lock_owned().await)
}

/// A parsed CLI event (mirrors cli/src/output.rs CliEvent).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliEvent {
    Progress { stage: String, percent: u8, message: String },
    Info { message: String },
    Complete { message: String },
    Error { message: String, #[serde(default)] code: Option<String> },
    Data { data: Value },
}

fn cli_binary_path() -> String {
    #[cfg(target_os = "windows")]
    let cli_name = "clawcli.exe";
    #[cfg(not(target_os = "windows"))]
    let cli_name = "clawcli";

    // 0. Explicit override — used during the v2 GUI migration so
    // developers can point the GUI at v2's debug binary without
    // touching install layout. See v2/docs/G-migration.md (G1-a).
    if let Ok(p) = std::env::var("CLAWCLI_BIN") {
        if !p.is_empty() {
            return p;
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        let dir = exe.parent().unwrap_or(std::path::Path::new("."));

        // 1. Exact name (dev mode: target/debug/ or target/release/)
        let exact = dir.join(cli_name);
        if exact.exists() {
            return exact.to_string_lossy().to_string();
        }

        // 2. Tauri sidecar: <cli_name>-<target-triple>[.exe] in same directory
        if let Ok(entries) = std::fs::read_dir(dir) {
            let prefix = "clawcli-";
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(prefix) && entry.path().is_file() {
                    return entry.path().to_string_lossy().to_string();
                }
            }
        }
    }

    cli_name.into()
}

fn new_cli_command(binary: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(binary);
    // tokio::process::Command has creation_flags directly on Windows —
    // no need to import std's CommandExt, which clippy flags as unused
    // (the method is resolved via tokio's inherent impl).
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x08000000);
    }
    cmd
}

/// Spawn a task to read stderr lines and log them via tracing.
/// Also sends a signal on each line to keep the idle timeout alive.
fn spawn_stderr_reader(
    stderr: tokio::process::ChildStderr,
    activity_tx: mpsc::Sender<()>,
) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut collected = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                tracing::debug!("[CLI stderr] {trimmed}");
                collected.push_str(trimmed);
                collected.push('\n');
                let _ = activity_tx.send(()).await;
            }
        }
        collected
    })
}

/// Run a CLI command and return the final result.
/// Reads both stdout (JSON events) and stderr (tracing logs).
/// Acquires per-instance mutex to prevent duplicate concurrent operations.
pub async fn run_cli(args: &[&str]) -> Result<Value> {
    let _instance_guard = acquire_instance_lock(args).await;
    let binary = cli_binary_path();
    let mut cmd = new_cli_command(&binary);
    cmd.arg("--json").args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn()
        .with_context(|| format!("Failed to run CLI: {} --json {}", binary, args.join(" ")))?;

    let stdout = child.stdout.take().context("No stdout")?;
    let stderr = child.stderr.take();

    // Merge stdout + stderr activity into a single idle timer
    let (activity_tx, mut activity_rx) = mpsc::channel::<()>(64);

    // Spawn stderr reader
    let stderr_task = stderr.map(|se| spawn_stderr_reader(se, activity_tx.clone()));

    // Read stdout JSON lines
    let stdout_activity = activity_tx.clone();
    let (line_tx, mut line_rx) = mpsc::channel::<String>(64);
    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = stdout_activity.send(()).await;
            if line_tx.send(line).await.is_err() { break; }
        }
    });
    drop(activity_tx);

    let idle_timeout = Duration::from_secs(IDLE_TIMEOUT_SECS);
    let mut last_data: Option<Value> = None;
    let mut last_complete: Option<String> = None;

    loop {
        tokio::select! {
            line = line_rx.recv() => {
                match line {
                    Some(line) => {
                        if let Ok(event) = serde_json::from_str::<CliEvent>(&line) {
                            match event {
                                CliEvent::Data { data } => last_data = Some(data),
                                CliEvent::Complete { message } => last_complete = Some(message),
                                CliEvent::Error { message, .. } => anyhow::bail!("{}", message),
                                _ => {}
                            }
                        }
                    }
                    None => break, // stdout closed
                }
            }
            _ = activity_rx.recv() => {
                // stderr activity — just keeps the timeout alive
            }
            _ = tokio::time::sleep(idle_timeout) => {
                child.kill().await.ok();
                anyhow::bail!(
                    "CLI command timed out — no output for {} minutes.",
                    IDLE_TIMEOUT_SECS / 60
                );
            }
        }
    }

    stdout_task.abort();
    if let Some(t) = stderr_task { t.abort(); }

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
/// All event types (Progress, Info, Complete, Error, Data) are forwarded.
/// stderr is captured and logged.
/// Acquires per-instance mutex to prevent duplicate concurrent operations.
///
/// `on_spawn` fires once the child is spawned, receiving its OS PID. Use
/// it to wire up cancel buttons: store the PID somewhere the cancel
/// handler can find, then send the child a SIGTERM / taskkill. Most
/// callers don't need this and can pass `|_| {}`.
pub async fn run_cli_streaming<F>(
    args: &[&str],
    tx: mpsc::Sender<CliEvent>,
    on_spawn: F,
) -> Result<Value>
where
    F: FnOnce(u32),
{
    run_cli_streaming_with_env(args, &[], tx, on_spawn).await
}

/// Same as `run_cli_streaming`, but injects additional environment variables
/// into the spawned child process. The parent's env is unchanged.
///
/// The install path uses this to pass the wizard-configured HTTP_PROXY /
/// HTTPS_PROXY / NO_PROXY into `clawcli` without persisting them to
/// config.toml — the proxy applies to this one install and goes away.
pub async fn run_cli_streaming_with_env<F>(
    args: &[&str],
    env_overrides: &[(&str, String)],
    tx: mpsc::Sender<CliEvent>,
    on_spawn: F,
) -> Result<Value>
where
    F: FnOnce(u32),
{
    let _instance_guard = acquire_instance_lock(args).await;
    let binary = cli_binary_path();
    let mut cmd = new_cli_command(&binary);
    cmd.arg("--json").args(args);
    for (k, v) in env_overrides {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn()
        .with_context(|| format!("Failed to spawn CLI: {} --json {}", binary, args.join(" ")))?;

    // Surface the child PID to the caller so they can kill it on cancel.
    // `child.id()` can return None only after the child exits, which hasn't
    // happened yet at this point.
    if let Some(pid) = child.id() {
        on_spawn(pid);
    }

    let stdout = child.stdout.take().context("No stdout")?;
    let stderr = child.stderr.take();

    let (activity_tx, mut activity_rx) = mpsc::channel::<()>(64);

    let stderr_task = stderr.map(|se| spawn_stderr_reader(se, activity_tx.clone()));

    let stdout_activity = activity_tx.clone();
    let (line_tx, mut line_rx) = mpsc::channel::<String>(64);
    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = stdout_activity.send(()).await;
            if line_tx.send(line).await.is_err() { break; }
        }
    });
    drop(activity_tx);

    let idle_timeout = Duration::from_secs(IDLE_TIMEOUT_SECS);
    let mut last_data: Option<Value> = None;
    let mut last_complete: Option<String> = None;
    let mut last_error: Option<String> = None;

    loop {
        tokio::select! {
            line = line_rx.recv() => {
                match line {
                    Some(line) => {
                        if let Ok(event) = serde_json::from_str::<CliEvent>(&line) {
                            match &event {
                                CliEvent::Data { data } => last_data = Some(data.clone()),
                                CliEvent::Complete { message } => last_complete = Some(message.clone()),
                                CliEvent::Error { message, .. } => last_error = Some(message.clone()),
                                _ => {}
                            }
                            let _ = tx.send(event).await;
                        }
                    }
                    None => break,
                }
            }
            _ = activity_rx.recv() => {}
            _ = tokio::time::sleep(idle_timeout) => {
                child.kill().await.ok();
                let err_msg = format!(
                    "Operation timed out — no progress for {} minutes.",
                    IDLE_TIMEOUT_SECS / 60
                );
                let _ = tx.send(CliEvent::Error {
                    message: err_msg.clone(),
                    code: Some("operation_stalled".into()),
                }).await;
                anyhow::bail!("{}", err_msg);
            }
        }
    }

    stdout_task.abort();
    if let Some(t) = stderr_task { t.abort(); }

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
