//! Hermes Agent CLI (NousResearch/hermes-agent).
//! Reference: hermes-agent.nousresearch.com/docs/reference/cli-commands

use std::time::Duration;

use async_trait::async_trait;

use crate::claw_ops::claw_cli::{ClawCli, DoctorOpts, LogsOpts, UpdateOpts};
use crate::common::CommandSpec;

const UPDATE_TIMEOUT_SECS: u64 = 15 * 60;
const DOCTOR_TIMEOUT_SECS: u64 = 60;
const CONFIG_TIMEOUT_SECS: u64 = 10;
const STATUS_TIMEOUT_SECS: u64 = 10;
const VERSION_TIMEOUT_SECS: u64 = 5;
const HELP_TIMEOUT_SECS: u64 = 5;
const LOGS_TIMEOUT_SECS: u64 = 30;

#[derive(Default)]
pub struct HermesCli;

impl HermesCli {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ClawCli for HermesCli {
    fn id(&self) -> &str { "hermes" }
    fn binary(&self) -> &str { "hermes" }
    fn supports_native(&self) -> bool { false }

    fn update(&self, opts: UpdateOpts) -> CommandSpec {
        let mut args = vec!["update".to_string()];
        if opts.non_interactive { args.push("--yes".into()); }
        CommandSpec::new(self.binary(), args)
            .with_timeout(Duration::from_secs(UPDATE_TIMEOUT_SECS))
    }

    fn doctor(&self, opts: DoctorOpts) -> CommandSpec {
        let mut args = vec!["doctor".to_string()];
        if opts.fix { args.push("--fix".into()); }
        CommandSpec::new(self.binary(), args)
            .with_timeout(Duration::from_secs(DOCTOR_TIMEOUT_SECS))
    }

    fn version(&self) -> CommandSpec {
        CommandSpec::new(self.binary(), ["version"])
            .with_timeout(Duration::from_secs(VERSION_TIMEOUT_SECS))
    }

    fn config_get(&self, key: &str) -> CommandSpec {
        CommandSpec::new(self.binary(), ["config", "get", key])
            .with_timeout(Duration::from_secs(CONFIG_TIMEOUT_SECS))
    }

    fn config_set(&self, key: &str, value: &str) -> CommandSpec {
        CommandSpec::new(self.binary(), ["config", "set", key, value])
            .with_timeout(Duration::from_secs(CONFIG_TIMEOUT_SECS))
    }

    fn config_list(&self) -> CommandSpec {
        CommandSpec::new(self.binary(), ["config", "list"])
            .with_timeout(Duration::from_secs(CONFIG_TIMEOUT_SECS))
    }

    fn logs(&self, opts: LogsOpts) -> CommandSpec {
        let mut args = vec!["logs".to_string()];
        if let Some(n) = opts.tail { args.push("--tail".into()); args.push(n.to_string()); }
        if opts.follow { args.push("--follow".into()); }
        if let Some(level) = &opts.level { args.push("--level".into()); args.push(level.clone()); }
        let mut s = CommandSpec::new(self.binary(), args);
        if !opts.follow { s = s.with_timeout(Duration::from_secs(LOGS_TIMEOUT_SECS)); }
        s
    }

    fn status(&self) -> CommandSpec {
        CommandSpec::new(self.binary(), ["status"])
            .with_timeout(Duration::from_secs(STATUS_TIMEOUT_SECS))
    }

    fn help(&self, subcommand: Option<&str>) -> CommandSpec {
        let args: Vec<String> = match subcommand {
            Some(s) => vec![s.to_string(), "--help".into()],
            None => vec!["--help".into()],
        };
        CommandSpec::new(self.binary(), args)
            .with_timeout(Duration::from_secs(HELP_TIMEOUT_SECS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::OutputFormat;

    #[test]
    fn identity() {
        let c = HermesCli::new();
        assert_eq!(c.id(), "hermes");
        assert_eq!(c.binary(), "hermes");
        assert!(!c.supports_native());
    }

    #[test]
    fn update_default() {
        let s = HermesCli::new().update(UpdateOpts::default());
        assert_eq!(s.args, vec!["update"]);
        assert_eq!(s.output_format, OutputFormat::Plain);
    }

    #[test]
    fn update_non_interactive() {
        let s = HermesCli::new().update(UpdateOpts { non_interactive: true, ..Default::default() });
        assert_eq!(s.args, vec!["update", "--yes"]);
    }

    #[test]
    fn update_ignores_openclaw_only_opts() {
        let s = HermesCli::new().update(UpdateOpts {
            json: true, dry_run: true, channel: Some("beta".into()),
            tag: Some("v1".into()), no_restart: true, ..Default::default()
        });
        assert_eq!(s.args, vec!["update"]);
    }

    #[test]
    fn doctor_with_fix() {
        let s = HermesCli::new().doctor(DoctorOpts { fix: true, ..Default::default() });
        assert_eq!(s.args, vec!["doctor", "--fix"]);
    }

    #[test]
    fn config_set_preserves_k_v() {
        let s = HermesCli::new().config_set("model.default", "anthropic/claude-sonnet");
        assert_eq!(s.args, vec!["config", "set", "model.default", "anthropic/claude-sonnet"]);
    }

    #[test]
    fn logs_follow_has_no_timeout() {
        let s = HermesCli::new().logs(LogsOpts { follow: true, ..Default::default() });
        assert!(s.timeout.is_none());
    }

    #[test]
    fn logs_tail_and_level() {
        let s = HermesCli::new().logs(LogsOpts { tail: Some(100), level: Some("error".into()), ..Default::default() });
        assert_eq!(s.args, vec!["logs", "--tail", "100", "--level", "error"]);
    }

    #[test]
    fn status_version_help() {
        let c = HermesCli::new();
        assert_eq!(c.status().args, vec!["status"]);
        assert_eq!(c.version().args, vec!["version"]);
        assert_eq!(c.help(None).args, vec!["--help"]);
        assert_eq!(c.help(Some("update")).args, vec!["update", "--help"]);
    }
}
