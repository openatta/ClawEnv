//! 执行过程中的错误类型。分类清晰便于上层分支处理。

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CommandError {
    /// 无法启动子进程（binary 不存在、权限不足等）。
    #[error("failed to spawn `{binary}`: {source}")]
    SpawnFailed {
        binary: String,
        #[source]
        source: std::io::Error,
    },

    /// 超时。子进程已被 kill。
    #[error("command timed out after {0:?}")]
    TimedOut(std::time::Duration),

    /// 被 `CancellationToken` 取消。子进程已被 kill。
    #[error("command was cancelled")]
    Cancelled,

    /// 进程退出但返回非零。`result` 保留了完整输出供上层审阅。
    #[error("command exited with code {exit_code}")]
    NonZeroExit {
        exit_code: i32,
        stdout: String,
        stderr: String,
    },

    /// 输出格式标记为 JsonFinal 但 stdout 不是合法 JSON。
    #[error("stdout is not valid JSON: {source}")]
    JsonParse {
        #[source]
        source: serde_json::Error,
        stdout: String,
    },

    /// 内部 I/O（读 stdout/stderr 管道失败）。
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Runner 实现特定错误（沙盒后端失败等）。
    #[error("runner error: {0}")]
    Runner(String),
}
