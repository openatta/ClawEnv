//! ClawOps — Claw CLI 编排层（Stage A）
//!
//! 独立模块，不接入现有业务逻辑（不改 manager/、不改 sandbox/）。
//! 提供两套抽象：
//! 1. `ClawCli` — 每个 Claw 产品（Hermes、OpenClaw）把自己的 CLI 子命令
//!    映射成 `CommandSpec`（纯数据，不执行）。
//! 2. `CommandRunner` — 把 `CommandSpec` 真正执行出去。两个 impl：
//!    - `LocalProcessRunner`：直接 tokio::process::Command（用于 native + 测试）
//!    - `SandboxBackendRunner`：委托给现有 `SandboxBackend`（用于三种沙盒）
//!
//! 详见 docs/25-claw-ops-stage-a.md。

pub mod cancel;
pub mod claw_cli;
pub mod cli;
pub mod command;
pub mod error;
pub mod event;
pub mod runner;
pub mod runners;

pub use cancel::CancellationToken;
pub use claw_cli::{ClawCli, DoctorOpts, LogsOpts, UpdateOpts};
pub use command::{CommandSpec, OutputFormat};
pub use error::CommandError;
pub use event::{ExecEvent, ExecResult};
pub use runner::CommandRunner;
