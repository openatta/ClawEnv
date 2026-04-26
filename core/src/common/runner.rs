//! `CommandRunner` — executes `CommandSpec`. Impls in `runners/`.

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::cancel::CancellationToken;
use super::command::CommandSpec;
use super::error::CommandError;
use super::event::{ExecEvent, ExecResult};

/// Run a command, tolerating "binary not installed" (spawn-failed) by
/// returning `Ok(None)`. Use this for read-only probes where a missing
/// host tool should be reported as "nothing to see here" rather than an
/// error.
///
/// All other errors propagate.
pub async fn try_exec(
    runner: &dyn CommandRunner,
    spec: CommandSpec,
    cancel: CancellationToken,
) -> Result<Option<ExecResult>, CommandError> {
    match runner.exec(spec, cancel).await {
        Ok(r) => Ok(Some(r)),
        Err(CommandError::SpawnFailed { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Identity for logging ("local" / "lima" / "wsl2" / "podman" / "mock").
    fn name(&self) -> &str;

    /// Run to completion, collect output. Non-zero exits come back as `Ok` with
    /// `exit_code != 0`; only runner-level failures (spawn / I/O / JSON parse)
    /// return `Err`. Cancel and timeout set the corresponding flags on
    /// `ExecResult` and return `Ok`.
    async fn exec(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> Result<ExecResult, CommandError>;

    /// Run and stream events. Returns a receiver; events arrive in order, with
    /// `Completed` last. Sentinel `Stderr("<cancelled>"/"<timed out>"/...)` is
    /// emitted before `Completed { exit_code: None }` on abnormal termination.
    fn exec_streaming(
        &self,
        spec: CommandSpec,
        cancel: CancellationToken,
    ) -> mpsc::Receiver<ExecEvent>;
}
