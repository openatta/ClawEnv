//! `SandboxBackendRunner` —— 委托给现有 `SandboxBackend`。
//!
//! 不改 `SandboxBackend` trait、不自己 spawn `limactl shell` 之类的进程，
//! 复用已经在三平台 × 三后端上验证过的命令执行路径。
//!
//! # 已知限制（详见 docs/25-claw-ops-stage-a.md §2.1）
//!
//! - **取消粒度有限**：`SandboxBackend::exec*` 是不可取消的，cancel token 触发时
//!   我们放弃 await，但沙盒里的子进程可能继续跑完。Stage B 讨论是否直接 bypass
//!   `SandboxBackend` 走 `limactl shell / wsl -d / podman exec`。
//! - **不支持 stdin 注入**：现有接口没有这条路。若 `CommandSpec.stdin.is_some()`，
//!   返回 `CommandError::Runner`。
//! - **stdout 和 stderr 不完全分离**：底层 `exec_with_progress` 把 stdout 作为返回值
//!   字符串、stderr 作为流式 channel。我们在完成后把整块 stdout 按行切开补发
//!   `ExecEvent::Stdout`（丢失了与 stderr 的精确交错，但对 JsonLines 来说没影响）。

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::claw_ops::cancel::CancellationToken;
use crate::claw_ops::command::{CommandSpec, OutputFormat};
use crate::claw_ops::error::CommandError;
use crate::claw_ops::event::{ExecEvent, ExecResult};
use crate::claw_ops::runner::CommandRunner;
use crate::platform::shell_quote;
use crate::sandbox::SandboxBackend;

pub struct SandboxBackendRunner {
    backend: Arc<dyn SandboxBackend>,
    name: String,
}

impl SandboxBackendRunner {
    pub fn new(backend: Arc<dyn SandboxBackend>) -> Self {
        let name = backend.name().to_string();
        Self { backend, name }
    }
}

/// 把 `CommandSpec` 渲染成 POSIX shell 命令字符串（Alpine 沙盒用 /bin/sh）。
/// 所有动态片段用 `shell_quote` 转义。
fn render_shell(spec: &CommandSpec) -> String {
    let mut parts: Vec<String> = Vec::new();

    // cd 前置
    if let Some(cwd) = &spec.cwd {
        parts.push(format!("cd {} &&", shell_quote(cwd)));
    }

    // env KEY=VAL ...
    if !spec.env.is_empty() {
        parts.push("env".into());
        for (k, v) in &spec.env {
            parts.push(format!("{}={}", k, shell_quote(v)));
        }
    }

    // binary 本身不 shell_quote —— 它是受控常量（"hermes" / "openclaw"），
    // 而且 quote 掉会让一些 shell 内建（如 "true"）行为变怪。
    // 但如果 binary 包含空格或特殊字符（仅测试场景），quote 它。
    if spec.binary.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) {
        parts.push(spec.binary.clone());
    } else {
        parts.push(shell_quote(&spec.binary));
    }

    for a in &spec.args {
        parts.push(shell_quote(a));
    }

    parts.join(" ")
}

#[async_trait]
impl CommandRunner for SandboxBackendRunner {
    fn name(&self) -> &str { &self.name }

    async fn exec(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> Result<ExecResult, CommandError> {
        if spec.stdin.is_some() {
            return Err(CommandError::Runner(
                "SandboxBackendRunner does not support stdin injection (Stage A limitation)".into(),
            ));
        }

        let start = Instant::now();
        let rendered = render_shell(&spec);
        let backend = self.backend.clone();
        let (stderr_tx, mut stderr_rx) = mpsc::channel::<String>(256);

        // 在独立任务里跑 exec_with_progress，方便 select! 包住取消/超时。
        let exec_task = tokio::spawn(async move {
            backend.exec_with_progress(&rendered, &stderr_tx).await
        });

        // 并发收集 stderr lines
        let stderr_collect = tokio::spawn(async move {
            let mut buf = String::new();
            while let Some(line) = stderr_rx.recv().await {
                buf.push_str(&line);
                buf.push('\n');
            }
            buf
        });

        // 等退出，同时监听 timeout / cancel
        let timeout_dur = spec.timeout;
        let wait_fut = async {
            let r = exec_task.await;
            match r {
                Ok(inner) => inner,
                Err(e) => Err(anyhow::anyhow!("task join: {e}")),
            }
        };

        let result: SandboxWait = tokio::select! {
            r = wait_fut => SandboxWait::Finished(r),
            _ = cancel.cancelled() => SandboxWait::Cancelled,
            _ = async {
                if let Some(d) = timeout_dur {
                    tokio::time::sleep(d).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => SandboxWait::TimedOut,
        };

        let stderr = stderr_collect.await.unwrap_or_default();

        match result {
            SandboxWait::Finished(Ok(stdout)) => {
                let structured = if spec.output_format == OutputFormat::JsonFinal {
                    match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            return Err(CommandError::JsonParse {
                                source: e,
                                stdout: stdout.clone(),
                            })
                        }
                    }
                } else {
                    None
                };
                Ok(ExecResult {
                    exit_code: 0,
                    stdout,
                    stderr,
                    structured,
                    duration: start.elapsed(),
                    was_cancelled: false,
                    was_timed_out: false,
                })
            }
            SandboxWait::Finished(Err(e)) => {
                // SandboxBackend 把"非零退出"也当 Err(anyhow)。我们把它原样塞进
                // ExecResult.exit_code=-1，并把错误信息放 stderr 里供上层诊断。
                let msg = format!("{e:#}");
                let merged_stderr = if stderr.is_empty() {
                    msg.clone()
                } else {
                    format!("{stderr}\n{msg}")
                };
                Ok(ExecResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: merged_stderr,
                    structured: None,
                    duration: start.elapsed(),
                    was_cancelled: false,
                    was_timed_out: false,
                })
            }
            SandboxWait::Cancelled => Ok(ExecResult {
                exit_code: -1,
                stdout: String::new(),
                stderr,
                structured: None,
                duration: start.elapsed(),
                was_cancelled: true,
                was_timed_out: false,
            }),
            SandboxWait::TimedOut => Ok(ExecResult {
                exit_code: -1,
                stdout: String::new(),
                stderr,
                structured: None,
                duration: start.elapsed(),
                was_cancelled: false,
                was_timed_out: true,
            }),
        }
    }

    fn exec_streaming(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> mpsc::Receiver<ExecEvent> {
        let (tx, rx) = mpsc::channel::<ExecEvent>(256);

        if spec.stdin.is_some() {
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let _ = tx2.send(ExecEvent::Stderr(
                    "<SandboxBackendRunner does not support stdin injection>".into(),
                )).await;
                let _ = tx2.send(ExecEvent::Completed { exit_code: None }).await;
            });
            return rx;
        }

        let backend = self.backend.clone();
        let rendered = render_shell(&spec);
        let is_jsonlines = spec.output_format == OutputFormat::JsonLines;
        let timeout_dur = spec.timeout;

        tokio::spawn(async move {
            let (stderr_tx, mut stderr_rx) = mpsc::channel::<String>(256);
            let tx_stderr = tx.clone();
            let stderr_forward = tokio::spawn(async move {
                while let Some(line) = stderr_rx.recv().await {
                    if tx_stderr.send(ExecEvent::Stderr(line)).await.is_err() {
                        break;
                    }
                }
            });

            let backend_for_exec = backend.clone();
            let rendered_for_exec = rendered.clone();
            let exec_handle = tokio::spawn(async move {
                backend_for_exec.exec_with_progress(&rendered_for_exec, &stderr_tx).await
            });

            let wait_fut = async {
                match exec_handle.await {
                    Ok(r) => r,
                    Err(e) => Err(anyhow::anyhow!("task join: {e}")),
                }
            };

            enum Outcome { Ok(String), Err(String), Cancelled, TimedOut }
            let outcome = tokio::select! {
                r = wait_fut => match r {
                    Ok(stdout) => Outcome::Ok(stdout),
                    Err(e) => Outcome::Err(format!("{e:#}")),
                },
                _ = cancel.cancelled() => Outcome::Cancelled,
                _ = async {
                    if let Some(d) = timeout_dur {
                        tokio::time::sleep(d).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => Outcome::TimedOut,
            };

            // 等 stderr 流收完（通常 exec_with_progress 返回时其内部发送端
            // 已关闭，stderr_forward 会自然退出）
            let _ = stderr_forward.await;

            match outcome {
                Outcome::Ok(stdout) => {
                    for line in stdout.lines() {
                        if is_jsonlines {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                                let _ = tx.send(ExecEvent::StructuredProgress(v)).await;
                            }
                        }
                        if tx.send(ExecEvent::Stdout(line.to_string())).await.is_err() {
                            break;
                        }
                    }
                    let _ = tx.send(ExecEvent::Completed { exit_code: Some(0) }).await;
                }
                Outcome::Err(msg) => {
                    let _ = tx.send(ExecEvent::Stderr(msg)).await;
                    let _ = tx.send(ExecEvent::Completed { exit_code: Some(-1) }).await;
                }
                Outcome::Cancelled => {
                    let _ = tx.send(ExecEvent::Stderr("<cancelled>".into())).await;
                    let _ = tx.send(ExecEvent::Completed { exit_code: None }).await;
                }
                Outcome::TimedOut => {
                    let _ = tx.send(ExecEvent::Stderr("<timed out>".into())).await;
                    let _ = tx.send(ExecEvent::Completed { exit_code: None }).await;
                }
            }
        });

        rx
    }
}

enum SandboxWait {
    Finished(anyhow::Result<String>),
    Cancelled,
    TimedOut,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_shell_basic() {
        let spec = CommandSpec::new("hermes", ["status"]);
        assert_eq!(render_shell(&spec), "hermes 'status'");
    }

    #[test]
    fn render_shell_escapes_args() {
        let spec = CommandSpec::new("hermes", ["config", "set", "key", "value with spaces"]);
        let s = render_shell(&spec);
        assert!(s.contains("'value with spaces'"), "got: {s}");
    }

    #[test]
    fn render_shell_escapes_single_quote() {
        let spec = CommandSpec::new("hermes", ["config", "set", "msg", "it's \"me\""]);
        let s = render_shell(&spec);
        // single quote should become '\''
        assert!(s.contains("'it'\\''s \"me\"'"), "got: {s}");
    }

    #[test]
    fn render_shell_prepends_cd_and_env() {
        let spec = CommandSpec::new("hermes", ["update"])
            .with_cwd("/opt/hermes")
            .with_env("HERMES_LOG", "debug");
        let s = render_shell(&spec);
        assert!(s.starts_with("cd '/opt/hermes' && env HERMES_LOG='debug' hermes "), "got: {s}");
    }
}
