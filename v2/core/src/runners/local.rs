//! `LocalProcessRunner` — tokio::process with full control surface.
//!
//! Supports every `CommandSpec` field: stdin, timeout, cancel, stdout/stderr
//! separation, JsonLines streaming, JsonFinal aggregate parse. The
//! grandchild-pipe-hang pitfall (SIGKILL of `sh` leaves `sleep` holding
//! stdout fds, blocking readers forever) is handled by aborting reader
//! tasks on force-kill paths.

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::common::{
    CancellationToken, CommandError, CommandRunner, CommandSpec, ExecEvent, ExecResult,
    OutputFormat,
};

pub struct LocalProcessRunner;

impl Default for LocalProcessRunner {
    fn default() -> Self { Self }
}

impl LocalProcessRunner {
    pub fn new() -> Self { Self }
}

struct RunOutcome {
    exit_code: i32,
    was_cancelled: bool,
    was_timed_out: bool,
}

async fn run(
    spec: CommandSpec,
    cancel: CancellationToken,
    events: mpsc::Sender<ExecEvent>,
) -> Result<RunOutcome, CommandError> {
    let mut cmd = tokio::process::Command::new(&spec.binary);
    cmd.args(&spec.args);
    for (k, v) in &spec.env { cmd.env(k, v); }
    if let Some(cwd) = &spec.cwd { cmd.current_dir(cwd); }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| CommandError::SpawnFailed {
        binary: spec.binary.clone(),
        source: e,
    })?;

    if let Some(input) = &spec.stdin {
        if let Some(mut stdin) = child.stdin.take() {
            let bytes = input.clone().into_bytes();
            let events_w = events.clone();
            tokio::spawn(async move {
                if let Err(e) = stdin.write_all(&bytes).await {
                    let _ = events_w.send(
                        ExecEvent::Stderr(format!("<stdin write failed: {e}>"))
                    ).await;
                }
                let _ = stdin.shutdown().await;
            });
        }
    } else {
        drop(child.stdin.take());
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let is_jsonlines = spec.output_format == OutputFormat::JsonLines;

    let stdout_task = stdout.map(|out| {
        let events = events.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(out).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if is_jsonlines {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                        if events.send(ExecEvent::StructuredProgress(v)).await.is_err() {
                            break;
                        }
                    }
                }
                if events.send(ExecEvent::Stdout(line)).await.is_err() { break; }
            }
        })
    });

    let stderr_task = stderr.map(|err| {
        let events = events.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(err).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if events.send(ExecEvent::Stderr(line)).await.is_err() { break; }
            }
        })
    });

    let timeout_duration = spec.timeout;
    enum WaitOutcome {
        Exited(std::io::Result<std::process::ExitStatus>),
        Cancelled,
        TimedOut,
    }
    let wait_result = tokio::select! {
        status = child.wait() => WaitOutcome::Exited(status),
        _ = cancel.cancelled() => WaitOutcome::Cancelled,
        _ = async {
            if let Some(d) = timeout_duration {
                tokio::time::sleep(d).await;
            } else {
                std::future::pending::<()>().await;
            }
        } => WaitOutcome::TimedOut,
    };

    let (outcome, force_killed) = match wait_result {
        WaitOutcome::Exited(Ok(status)) => (
            RunOutcome {
                exit_code: status.code().unwrap_or(-1),
                was_cancelled: false,
                was_timed_out: false,
            },
            false,
        ),
        WaitOutcome::Exited(Err(e)) => return Err(CommandError::Io(e)),
        WaitOutcome::Cancelled => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            (
                RunOutcome { exit_code: -1, was_cancelled: true, was_timed_out: false },
                true,
            )
        }
        WaitOutcome::TimedOut => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            (
                RunOutcome { exit_code: -1, was_cancelled: false, was_timed_out: true },
                true,
            )
        }
    };

    // On force-kill paths, grandchildren may still hold the stdout/stderr
    // pipes open (classic `sh -c 'while :; do sleep 3600; done'` pitfall).
    // Abort reader tasks so we don't wait for EOF that will never come.
    match (stdout_task, stderr_task, force_killed) {
        (Some(t_out), Some(t_err), false) => {
            let _ = t_out.await; let _ = t_err.await;
        }
        (Some(t_out), Some(t_err), true) => {
            t_out.abort(); t_err.abort();
            let _ = t_out.await; let _ = t_err.await;
        }
        (Some(t), None, false) => { let _ = t.await; }
        (Some(t), None, true)  => { t.abort(); let _ = t.await; }
        (None, Some(t), false) => { let _ = t.await; }
        (None, Some(t), true)  => { t.abort(); let _ = t.await; }
        (None, None, _) => {}
    }

    Ok(outcome)
}

#[async_trait]
impl CommandRunner for LocalProcessRunner {
    fn name(&self) -> &str { "local" }

    async fn exec(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> Result<ExecResult, CommandError> {
        let start = Instant::now();
        let (tx, mut rx) = mpsc::channel::<ExecEvent>(256);
        let spec_r = spec.clone();
        let cancel_r = cancel.clone();
        let handle = tokio::spawn(async move { run(spec_r, cancel_r, tx).await });

        let mut stdout = String::new();
        let mut stderr = String::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                ExecEvent::Stdout(l) => { stdout.push_str(&l); stdout.push('\n'); }
                ExecEvent::Stderr(l) => { stderr.push_str(&l); stderr.push('\n'); }
                ExecEvent::StructuredProgress(_) | ExecEvent::Completed { .. } => {}
            }
        }

        let outcome = match handle.await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(CommandError::Runner(format!("task join: {e}"))),
        };

        let structured = if spec.output_format == OutputFormat::JsonFinal
            && outcome.exit_code == 0
            && !outcome.was_cancelled
            && !outcome.was_timed_out
        {
            match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                Ok(v) => Some(v),
                Err(e) => return Err(CommandError::JsonParse { source: e, stdout }),
            }
        } else {
            None
        };

        Ok(ExecResult {
            exit_code: outcome.exit_code,
            stdout, stderr, structured,
            duration: start.elapsed(),
            was_cancelled: outcome.was_cancelled,
            was_timed_out: outcome.was_timed_out,
        })
    }

    fn exec_streaming(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> mpsc::Receiver<ExecEvent> {
        let (tx, rx) = mpsc::channel::<ExecEvent>(256);
        tokio::spawn(async move {
            let tx_final = tx.clone();
            let result = run(spec, cancel, tx).await;
            let exit_code = match result {
                Ok(o) if !o.was_cancelled && !o.was_timed_out => Some(o.exit_code),
                Ok(o) => {
                    let msg = if o.was_timed_out { "<timed out>" } else { "<cancelled>" };
                    let _ = tx_final.send(ExecEvent::Stderr(msg.into())).await;
                    None
                }
                Err(CommandError::SpawnFailed { binary, source }) => {
                    let _ = tx_final.send(ExecEvent::Stderr(
                        format!("<spawn failed for {binary}: {source}>"),
                    )).await;
                    None
                }
                Err(e) => {
                    let _ = tx_final.send(ExecEvent::Stderr(format!("<runner error: {e}>"))).await;
                    None
                }
            };
            let _ = tx_final.send(ExecEvent::Completed { exit_code }).await;
        });
        rx
    }
}
