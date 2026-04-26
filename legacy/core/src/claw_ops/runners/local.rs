//! `LocalProcessRunner` —— 直接通过 `tokio::process::Command` 启动子进程。
//!
//! 支持所有 `CommandSpec` 能力：stdin 注入、超时、取消、分离 stdout/stderr、
//! JsonLines 流式解析、JsonFinal 整体解析。
//!
//! 这是底层 runner，native 模式和所有集成测试都使用它。

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::claw_ops::cancel::CancellationToken;
use crate::claw_ops::command::{CommandSpec, OutputFormat};
use crate::claw_ops::error::CommandError;
use crate::claw_ops::event::{ExecEvent, ExecResult};
use crate::claw_ops::runner::CommandRunner;

pub struct LocalProcessRunner;

impl Default for LocalProcessRunner {
    fn default() -> Self { Self }
}

impl LocalProcessRunner {
    pub fn new() -> Self { Self }
}

/// 内部 run() 的结果。
struct RunOutcome {
    exit_code: i32,
    was_cancelled: bool,
    was_timed_out: bool,
}

/// 驱动一次子进程执行，把所有过程事件推入 `events` channel。
/// 返回时 `events` 会被 drop（channel 关闭），调用方可以通过 `rx.recv()` 的
/// `None` 来感知进程结束。
async fn run(
    spec: CommandSpec,
    cancel: CancellationToken,
    events: mpsc::Sender<ExecEvent>,
) -> Result<RunOutcome, CommandError> {
    // ---- 1. 构造 Command ----
    let mut cmd = tokio::process::Command::new(&spec.binary);
    cmd.args(&spec.args);
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    // ---- 2. spawn ----
    let mut child = cmd.spawn().map_err(|e| CommandError::SpawnFailed {
        binary: spec.binary.clone(),
        source: e,
    })?;

    // ---- 3. 写 stdin（若需要）----
    if let Some(input) = &spec.stdin {
        if let Some(mut stdin) = child.stdin.take() {
            // 即使写失败也不直接 bail —— 进程可能只读部分就退出了。
            // 把写入放进独立任务，失败记到 stderr 事件里供调试。
            let input_bytes = input.clone().into_bytes();
            let events_stdin = events.clone();
            tokio::spawn(async move {
                if let Err(e) = stdin.write_all(&input_bytes).await {
                    let _ = events_stdin.send(ExecEvent::Stderr(
                        format!("<stdin write failed: {e}>"),
                    )).await;
                }
                let _ = stdin.shutdown().await;
            });
        }
    } else {
        // 主动关掉 stdin，避免子进程 read 阻塞。
        drop(child.stdin.take());
    }

    // ---- 4. 读 stdout / stderr ----
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
                if events.send(ExecEvent::Stdout(line)).await.is_err() {
                    break;
                }
            }
        })
    });

    let stderr_task = stderr.map(|err| {
        let events = events.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(err).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if events.send(ExecEvent::Stderr(line)).await.is_err() {
                    break;
                }
            }
        })
    });

    // ---- 5. 等待退出，同时监听 cancel + timeout ----
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

    // ---- 6. 等读管道任务收尾 ----
    //
    // 正常退出时，pipe 会自然关闭，reader 任务会看到 EOF 并退出——await 它们
    // 是为了确保所有 stdout/stderr 行都已进 channel。
    //
    // 强制 kill 时不能 await —— 当被 kill 的进程是 shell 且有孙子进程
    // （典型例子：`sh -c 'while :; do sleep 3600; done'`），SIGKILL 只打到 shell，
    // `sleep` 孙子继承了 stdout/stderr fd 并继续存活，reader 会永久阻塞在
    // next_line() 上。这里 abort 读任务，丢弃残余输出（已经 kill 了，反正也没啥）。
    match (stdout_task, stderr_task, force_killed) {
        (Some(t_out), Some(t_err), false) => {
            let _ = t_out.await;
            let _ = t_err.await;
        }
        (Some(t_out), Some(t_err), true) => {
            t_out.abort();
            t_err.abort();
            let _ = t_out.await;
            let _ = t_err.await;
        }
        (Some(t), None, false) => { let _ = t.await; }
        (Some(t), None, true) => { t.abort(); let _ = t.await; }
        (None, Some(t), false) => { let _ = t.await; }
        (None, Some(t), true) => { t.abort(); let _ = t.await; }
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

        let spec_for_run = spec.clone();
        let cancel_for_run = cancel.clone();
        let run_handle = tokio::spawn(async move {
            run(spec_for_run, cancel_for_run, tx).await
        });

        let mut stdout = String::new();
        let mut stderr = String::new();
        while let Some(ev) = rx.recv().await {
            match ev {
                ExecEvent::Stdout(line) => {
                    stdout.push_str(&line);
                    stdout.push('\n');
                }
                ExecEvent::Stderr(line) => {
                    stderr.push_str(&line);
                    stderr.push('\n');
                }
                ExecEvent::StructuredProgress(_) | ExecEvent::Completed { .. } => {}
            }
        }

        let outcome = match run_handle.await {
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
            stdout,
            stderr,
            structured,
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
                Ok(outcome) if !outcome.was_cancelled && !outcome.was_timed_out => {
                    Some(outcome.exit_code)
                }
                Ok(outcome) => {
                    let msg = if outcome.was_timed_out {
                        "<timed out>"
                    } else {
                        "<cancelled>"
                    };
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
