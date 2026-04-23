//! `SandboxCommandRunner` — wraps a v2 `SandboxBackend` to satisfy the
//! `CommandRunner` trait. Used by ClawOps `--execute` to run a claw CLI
//! inside the sandbox (e.g. `hermes update` inside Lima).
//!
//! Limitations (same category as v1's adapter):
//! - Cancel is best-effort (the backend.exec() call is atomic; we abandon
//!   the await but the in-sandbox process may linger).
//! - No stdin injection — users requiring prompts should either pre-answer
//!   via --yes flags, or run via native (host) execution.
//! - stdout/stderr interleaving is lost — backend.exec returns stdout as a
//!   string; we never see per-line stderr. JsonLines / JsonFinal still
//!   work because they're parsed from the full stdout blob after exit.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::common::{
    CancellationToken, CommandError, CommandRunner, CommandSpec, ExecEvent, ExecResult,
    OutputFormat,
};
use crate::sandbox_backend::SandboxBackend;

pub struct SandboxCommandRunner {
    backend: Arc<dyn SandboxBackend>,
    name: String,
}

impl SandboxCommandRunner {
    pub fn new(backend: Arc<dyn SandboxBackend>) -> Self {
        let name = format!("{} ({})", backend.name(), backend.instance());
        Self { backend, name }
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn render_shell(spec: &CommandSpec) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(cwd) = &spec.cwd {
        parts.push(format!("cd {} &&", shell_quote(cwd)));
    }
    if !spec.env.is_empty() {
        parts.push("env".into());
        for (k, v) in &spec.env {
            parts.push(format!("{}={}", k, shell_quote(v)));
        }
    }
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
impl CommandRunner for SandboxCommandRunner {
    fn name(&self) -> &str { &self.name }

    async fn exec(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> Result<ExecResult, CommandError> {
        if spec.stdin.is_some() {
            return Err(CommandError::Runner(
                "SandboxCommandRunner does not support stdin injection".into(),
            ));
        }
        let start = Instant::now();
        let rendered = render_shell(&spec);
        let backend = self.backend.clone();
        let timeout_dur = spec.timeout;

        let exec_fut = tokio::spawn(async move { backend.exec(&rendered).await });

        enum Outcome {
            Ok(String),
            Err(String),
            Cancelled,
            TimedOut,
        }
        let outcome = tokio::select! {
            r = exec_fut => match r {
                Ok(Ok(stdout)) => Outcome::Ok(stdout),
                Ok(Err(e)) => Outcome::Err(format!("{e:#}")),
                Err(e) => Outcome::Err(format!("task join: {e}")),
            },
            _ = cancel.cancelled() => Outcome::Cancelled,
            _ = async {
                if let Some(d) = timeout_dur { tokio::time::sleep(d).await; }
                else { std::future::pending::<()>().await; }
            } => Outcome::TimedOut,
        };

        match outcome {
            Outcome::Ok(stdout) => {
                let structured = if spec.output_format == OutputFormat::JsonFinal {
                    match serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                        Ok(v) => Some(v),
                        Err(e) => return Err(CommandError::JsonParse {
                            source: e, stdout: stdout.clone(),
                        }),
                    }
                } else { None };
                Ok(ExecResult {
                    exit_code: 0, stdout, stderr: String::new(), structured,
                    duration: start.elapsed(),
                    was_cancelled: false, was_timed_out: false,
                })
            }
            Outcome::Err(msg) => Ok(ExecResult {
                exit_code: -1, stdout: String::new(), stderr: msg, structured: None,
                duration: start.elapsed(),
                was_cancelled: false, was_timed_out: false,
            }),
            Outcome::Cancelled => Ok(ExecResult {
                exit_code: -1, stdout: String::new(), stderr: String::new(),
                structured: None, duration: start.elapsed(),
                was_cancelled: true, was_timed_out: false,
            }),
            Outcome::TimedOut => Ok(ExecResult {
                exit_code: -1, stdout: String::new(), stderr: String::new(),
                structured: None, duration: start.elapsed(),
                was_cancelled: false, was_timed_out: true,
            }),
        }
    }

    fn exec_streaming(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> mpsc::Receiver<ExecEvent> {
        let (tx, rx) = mpsc::channel::<ExecEvent>(256);
        let me = SandboxCommandRunner {
            backend: self.backend.clone(),
            name: self.name.clone(),
        };
        tokio::spawn(async move {
            let is_jsonlines = spec.output_format == OutputFormat::JsonLines;
            let result = me.exec(spec, cancel).await;
            match result {
                Ok(res) => {
                    for line in res.stdout.lines() {
                        if is_jsonlines {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                                let _ = tx.send(ExecEvent::StructuredProgress(v)).await;
                            }
                        }
                        if tx.send(ExecEvent::Stdout(line.to_string())).await.is_err() { break; }
                    }
                    if !res.stderr.is_empty() {
                        let _ = tx.send(ExecEvent::Stderr(res.stderr)).await;
                    }
                    let exit = if res.was_cancelled || res.was_timed_out {
                        None
                    } else {
                        Some(res.exit_code)
                    };
                    let _ = tx.send(ExecEvent::Completed { exit_code: exit }).await;
                }
                Err(e) => {
                    let _ = tx.send(ExecEvent::Stderr(format!("<runner error: {e}>"))).await;
                    let _ = tx.send(ExecEvent::Completed { exit_code: None }).await;
                }
            }
        });
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::CommandSpec;

    #[test]
    fn render_basic() {
        let s = CommandSpec::new("hermes", ["status"]);
        assert_eq!(render_shell(&s), "hermes 'status'");
    }

    #[test]
    fn render_escapes_args() {
        let s = CommandSpec::new("hermes", ["config", "set", "k", "v w"]);
        assert!(render_shell(&s).contains("'v w'"));
    }

    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }
}
