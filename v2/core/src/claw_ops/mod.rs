//! ClawOps — per-Claw CLI command mapping.
//!
//! Each Claw product (Hermes, OpenClaw) implements `ClawCli` and returns
//! `CommandSpec` values for each subcommand. Execution is delegated to any
//! `CommandRunner`.

pub mod claw_cli;
pub mod hermes;
pub mod openclaw;
pub mod registry;

pub use claw_cli::{ClawCli, DoctorOpts, LogsOpts, UpdateOpts};
pub use hermes::HermesCli;
pub use openclaw::OpenClawCli;
pub use registry::ClawRegistry;
