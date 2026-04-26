//! `ClawCli` trait and option types.

use async_trait::async_trait;

use crate::common::CommandSpec;

#[derive(Debug, Clone, Default)]
pub struct UpdateOpts {
    pub non_interactive: bool,
    pub json: bool,
    pub dry_run: bool,
    pub channel: Option<String>,
    pub tag: Option<String>,
    pub no_restart: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DoctorOpts {
    pub fix: bool,
    pub json: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LogsOpts {
    pub tail: Option<u32>,
    pub follow: bool,
    pub level: Option<String>,
}

#[async_trait]
pub trait ClawCli: Send + Sync {
    fn id(&self) -> &str;
    fn binary(&self) -> &str;
    fn supports_native(&self) -> bool;

    fn update(&self, opts: UpdateOpts) -> CommandSpec;
    fn doctor(&self, opts: DoctorOpts) -> CommandSpec;
    fn version(&self) -> CommandSpec;

    fn config_get(&self, key: &str) -> CommandSpec;
    fn config_set(&self, key: &str, value: &str) -> CommandSpec;
    fn config_list(&self) -> CommandSpec;

    fn logs(&self, opts: LogsOpts) -> CommandSpec;
    fn status(&self) -> CommandSpec;
    fn help(&self, subcommand: Option<&str>) -> CommandSpec;
}
