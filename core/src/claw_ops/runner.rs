//! `CommandRunner` trait —— 把 `CommandSpec` 真正执行出去。
//!
//! 两个实现位于 `runners/`：
//! - `LocalProcessRunner`：本地 tokio::process，支持全功能（stdin / timeout / cancel / 分离 stdout+stderr）。
//! - `SandboxBackendRunner`：委托给现有 `SandboxBackend`，受限于其 `exec`/`exec_with_progress` 接口。

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::cancel::CancellationToken;
use super::command::CommandSpec;
use super::error::CommandError;
use super::event::{ExecEvent, ExecResult};

#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Runner 识别名，用于日志与跟踪（`"local"` / `"lima"` / `"wsl2"` / `"podman"` / `"mock"`）。
    fn name(&self) -> &str;

    /// 一次性执行，收集完整输出。
    ///
    /// 即使退出码非零，也会以 `Ok(ExecResult)` 返回（`exit_code` 非 0）——
    /// 让上层决定是否把非零退出当错误。只有 spawn 失败、超时、取消、I/O 错误
    /// 以 `Err` 返回。
    async fn exec(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> Result<ExecResult, CommandError>;

    /// 流式执行。返回一个 `mpsc::Receiver<ExecEvent>`，调用方 `.recv().await` 消费。
    ///
    /// 事件顺序：
    /// - 按产生顺序穿插 `Stdout` / `Stderr` / `StructuredProgress`（JsonLines 模式）
    /// - 最后一个事件是 `Completed { exit_code }`
    /// - 取消/超时/spawn 失败时，最后事件仍是 `Completed { exit_code: None }`，
    ///   同时会多一条 `Stderr("<cancelled/timeout/spawn_failed: ...>")` 说明原因
    fn exec_streaming(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> mpsc::Receiver<ExecEvent>;
}
