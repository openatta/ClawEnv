//! clawops-core — ClawEnv v2 Ops abstractions.
//!
//! See `v2/docs/DESIGN.md` for the full architecture.
//!
//! Layout:
//!
//! - `common` — CommandSpec / CommandRunner / CancellationToken / ProgressSink / OpsError
//! - `runners` — LocalProcessRunner (tokio::process-based)
//! - `adapters` — bridges to v1 (SandboxBackendRunner)
//! - `extract` — tar.gz / tar.xz / zip extraction
//! - `claw_ops` — ClawCli trait + HermesCli + OpenClawCli
//! - `sandbox_ops` — SandboxOps trait + Lima/Wsl/Podman impls
//! - `native_ops` — NativeOps trait + DefaultNativeOps
//! - `download_ops` — DownloadOps trait + catalog + fetch

pub mod common;
pub mod paths;
pub mod runners;
pub mod sandbox_backend;
pub mod extract;
pub mod claw_ops;
pub mod sandbox_ops;
pub mod native_ops;
pub mod download_ops;
pub mod instance;
pub mod preflight;
pub mod credentials;
pub mod proxy;
pub mod provisioning;
pub mod config_loader;
pub mod launcher;
pub mod update;

// Re-exports for convenience.
pub use common::{
    CancellationToken, CommandError, CommandRunner, CommandSpec, ExecEvent, ExecResult,
    OpsError, OutputFormat, ProgressEvent, ProgressSink,
};
