//! Execution-time events and final result.

use std::time::Duration;

#[derive(Debug, Clone)]
pub enum ExecEvent {
    Stdout(String),
    Stderr(String),
    /// JsonLines mode: each successful `serde_json::from_str` on a stdout line.
    StructuredProgress(serde_json::Value),
    /// End-of-stream. `None` = killed (cancel/timeout/spawn-fail).
    Completed { exit_code: Option<i32> },
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// `OutputFormat::JsonFinal`: full stdout parsed as one JSON.
    pub structured: Option<serde_json::Value>,
    pub duration: Duration,
    pub was_cancelled: bool,
    pub was_timed_out: bool,
}

impl ExecResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0 && !self.was_cancelled && !self.was_timed_out
    }
}
