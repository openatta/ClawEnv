//! OpenClaw CLI. Reference: docs.openclaw.ai/cli/*

use std::time::Duration;

use async_trait::async_trait;

use crate::claw_ops::claw_cli::{ClawCli, DoctorOpts, LogsOpts, UpdateOpts};
use crate::common::{CommandSpec, OutputFormat};

const UPDATE_TIMEOUT_SECS: u64 = 10 * 60;
const DOCTOR_TIMEOUT_SECS: u64 = 60;
const CONFIG_TIMEOUT_SECS: u64 = 10;
const STATUS_TIMEOUT_SECS: u64 = 10;
const VERSION_TIMEOUT_SECS: u64 = 5;
const HELP_TIMEOUT_SECS: u64 = 5;
const LOGS_TIMEOUT_SECS: u64 = 30;

#[derive(Default)]
pub struct OpenClawCli;

impl OpenClawCli {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ClawCli for OpenClawCli {
    fn id(&self) -> &str { "openclaw" }
    fn binary(&self) -> &str { "openclaw" }
    fn supports_native(&self) -> bool { true }

    fn update(&self, opts: UpdateOpts) -> CommandSpec {
        let mut args = vec!["update".to_string()];
        if opts.non_interactive { args.push("--yes".into()); }
        if opts.dry_run { args.push("--dry-run".into()); }
        if opts.no_restart { args.push("--no-restart".into()); }
        if let Some(ch) = &opts.channel { args.push("--channel".into()); args.push(ch.clone()); }
        if let Some(t) = &opts.tag { args.push("--tag".into()); args.push(t.clone()); }
        let mut fmt = OutputFormat::Plain;
        if opts.json { args.push("--json".into()); fmt = OutputFormat::JsonFinal; }
        CommandSpec::new(self.binary(), args)
            .with_timeout(Duration::from_secs(UPDATE_TIMEOUT_SECS))
            .with_output_format(fmt)
    }

    fn doctor(&self, opts: DoctorOpts) -> CommandSpec {
        let mut args = vec!["doctor".to_string()];
        let mut fmt = OutputFormat::Plain;
        if opts.json { args.push("--json".into()); fmt = OutputFormat::JsonFinal; }
        CommandSpec::new(self.binary(), args)
            .with_timeout(Duration::from_secs(DOCTOR_TIMEOUT_SECS))
            .with_output_format(fmt)
    }

    fn version(&self) -> CommandSpec {
        CommandSpec::new(self.binary(), ["--version"])
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
        if let Some(l) = &opts.level { args.push("--level".into()); args.push(l.clone()); }
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

    #[test]
    fn identity() {
        let c = OpenClawCli::new();
        assert_eq!(c.id(), "openclaw");
        assert!(c.supports_native());
    }

    #[test]
    fn update_default_plain() {
        let s = OpenClawCli::new().update(UpdateOpts::default());
        assert_eq!(s.args, vec!["update"]);
        assert_eq!(s.output_format, OutputFormat::Plain);
    }

    #[test]
    fn update_json_switches_to_jsonfinal() {
        let s = OpenClawCli::new().update(UpdateOpts { json: true, ..Default::default() });
        assert!(s.args.contains(&"--json".into()));
        assert_eq!(s.output_format, OutputFormat::JsonFinal);
    }

    #[test]
    fn update_all_flags_order() {
        let s = OpenClawCli::new().update(UpdateOpts {
            non_interactive: true, json: true, dry_run: true,
            channel: Some("beta".into()), tag: Some("2026.4.5".into()),
            no_restart: true,
        });
        assert_eq!(s.args, vec![
            "update", "--yes", "--dry-run", "--no-restart",
            "--channel", "beta", "--tag", "2026.4.5", "--json",
        ]);
    }

    #[test]
    fn doctor_json() {
        let s = OpenClawCli::new().doctor(DoctorOpts { json: true, ..Default::default() });
        assert_eq!(s.args, vec!["doctor", "--json"]);
        assert_eq!(s.output_format, OutputFormat::JsonFinal);
    }

    #[test]
    fn version_uses_top_level_flag() {
        let s = OpenClawCli::new().version();
        assert_eq!(s.args, vec!["--version"]);
    }

    #[test]
    fn config_list() {
        let s = OpenClawCli::new().config_list();
        assert_eq!(s.args, vec!["config", "list"]);
    }

    #[test]
    fn logs_follow_no_timeout() {
        let s = OpenClawCli::new().logs(LogsOpts { follow: true, ..Default::default() });
        assert!(s.timeout.is_none());
    }
}
