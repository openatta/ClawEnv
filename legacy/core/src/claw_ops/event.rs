//! 执行过程事件与最终结果。

use std::time::Duration;

/// 流式执行过程中产生的事件。
#[derive(Debug, Clone)]
pub enum ExecEvent {
    /// stdout 一行（已去掉行尾 `\n`）。
    Stdout(String),
    /// stderr 一行（已去掉行尾 `\n`）。
    Stderr(String),
    /// `OutputFormat::JsonLines` 下，每行 stdout 被解析成 JSON 后通过此事件抛出。
    /// 原始 `Stdout(line)` 事件仍然会发送，调用方可以各取所需。
    StructuredProgress(serde_json::Value),
    /// 进程结束。`None` 表示被信号杀掉（取消/超时）。
    Completed { exit_code: Option<i32> },
}

/// 一次性执行的完整结果。
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// `OutputFormat::JsonFinal` 下，整体 stdout 解析出来的 JSON。
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
