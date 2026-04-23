//! OpenClaw CLI 映射。
//!
//! 基于 `docs.openclaw.ai/cli/*` 的官方参考。
//! 核实的子命令：update (含 --channel/--tag/--dry-run/--json/--no-restart/--yes/--timeout) /
//! doctor / config / gateway / status / ...

use std::time::Duration;

use async_trait::async_trait;

use crate::claw_ops::claw_cli::{ClawCli, DoctorOpts, LogsOpts, UpdateOpts};
use crate::claw_ops::command::{CommandSpec, OutputFormat};

const UPDATE_TIMEOUT_SECS: u64 = 10 * 60;   // 10 min
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
        if opts.non_interactive {
            args.push("--yes".into());
        }
        if opts.dry_run {
            args.push("--dry-run".into());
        }
        if opts.no_restart {
            args.push("--no-restart".into());
        }
        if let Some(ch) = &opts.channel {
            args.push("--channel".into());
            args.push(ch.clone());
        }
        if let Some(tag) = &opts.tag {
            args.push("--tag".into());
            args.push(tag.clone());
        }
        let mut format = OutputFormat::Plain;
        if opts.json {
            args.push("--json".into());
            // openclaw update --json 输出一次性完整 JSON（UpdateRunResult）
            format = OutputFormat::JsonFinal;
        }
        CommandSpec::new(self.binary(), args)
            .with_timeout(Duration::from_secs(UPDATE_TIMEOUT_SECS))
            .with_output_format(format)
    }

    fn doctor(&self, opts: DoctorOpts) -> CommandSpec {
        let mut args = vec!["doctor".to_string()];
        // OpenClaw doctor 官方 docs 目前未公开 --fix 语义（与 Hermes 不同），
        // 这里把 opts.fix 留给未来：若上游加上，我们再 map。
        let mut format = OutputFormat::Plain;
        if opts.json {
            args.push("--json".into());
            format = OutputFormat::JsonFinal;
        }
        CommandSpec::new(self.binary(), args)
            .with_timeout(Duration::from_secs(DOCTOR_TIMEOUT_SECS))
            .with_output_format(format)
    }

    fn version(&self) -> CommandSpec {
        // `openclaw --version` 是 top-level flag，不是子命令
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
        if let Some(n) = opts.tail {
            args.push("--tail".into());
            args.push(n.to_string());
        }
        if opts.follow {
            args.push("--follow".into());
        }
        if let Some(level) = &opts.level {
            args.push("--level".into());
            args.push(level.clone());
        }
        let mut spec = CommandSpec::new(self.binary(), args);
        if !opts.follow {
            spec = spec.with_timeout(Duration::from_secs(LOGS_TIMEOUT_SECS));
        }
        spec
    }

    fn status(&self) -> CommandSpec {
        CommandSpec::new(self.binary(), ["status"])
            .with_timeout(Duration::from_secs(STATUS_TIMEOUT_SECS))
    }

    fn help(&self, subcommand: Option<&str>) -> CommandSpec {
        let args: Vec<String> = match subcommand {
            Some(sub) => vec![sub.to_string(), "--help".into()],
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
    fn id_and_binary() {
        let cli = OpenClawCli::new();
        assert_eq!(cli.id(), "openclaw");
        assert_eq!(cli.binary(), "openclaw");
        assert!(cli.supports_native());
    }

    #[test]
    fn update_default_opts_has_no_flags() {
        let spec = OpenClawCli::new().update(UpdateOpts::default());
        assert_eq!(spec.args, vec!["update"]);
        assert_eq!(spec.output_format, OutputFormat::Plain);
    }

    #[test]
    fn update_json_switches_format_to_jsonfinal() {
        let spec = OpenClawCli::new().update(UpdateOpts { json: true, ..Default::default() });
        assert!(spec.args.contains(&"--json".to_string()));
        assert_eq!(spec.output_format, OutputFormat::JsonFinal);
    }

    #[test]
    fn update_all_flags_in_order() {
        let spec = OpenClawCli::new().update(UpdateOpts {
            non_interactive: true,
            json: true,
            dry_run: true,
            channel: Some("beta".into()),
            tag: Some("2026.4.5".into()),
            no_restart: true,
        });
        // 期望顺序：update --yes --dry-run --no-restart --channel beta --tag 2026.4.5 --json
        assert_eq!(
            spec.args,
            vec![
                "update",
                "--yes",
                "--dry-run",
                "--no-restart",
                "--channel",
                "beta",
                "--tag",
                "2026.4.5",
                "--json",
            ]
        );
    }

    #[test]
    fn update_channel_only() {
        let spec = OpenClawCli::new().update(UpdateOpts {
            channel: Some("dev".into()),
            ..Default::default()
        });
        assert_eq!(spec.args, vec!["update", "--channel", "dev"]);
    }

    #[test]
    fn doctor_default() {
        let spec = OpenClawCli::new().doctor(DoctorOpts::default());
        assert_eq!(spec.args, vec!["doctor"]);
        assert_eq!(spec.output_format, OutputFormat::Plain);
    }

    #[test]
    fn doctor_json() {
        let spec = OpenClawCli::new().doctor(DoctorOpts { json: true, ..Default::default() });
        assert_eq!(spec.args, vec!["doctor", "--json"]);
        assert_eq!(spec.output_format, OutputFormat::JsonFinal);
    }

    #[test]
    fn version_uses_top_level_flag() {
        let spec = OpenClawCli::new().version();
        assert_eq!(spec.args, vec!["--version"]);
    }

    #[test]
    fn config_set_preserves_key_and_value() {
        let spec = OpenClawCli::new().config_set("gateway.mode", "local");
        assert_eq!(spec.args, vec!["config", "set", "gateway.mode", "local"]);
    }

    #[test]
    fn logs_with_tail() {
        let spec = OpenClawCli::new().logs(LogsOpts { tail: Some(50), ..Default::default() });
        assert_eq!(spec.args, vec!["logs", "--tail", "50"]);
    }

    #[test]
    fn logs_follow_no_timeout() {
        let spec = OpenClawCli::new().logs(LogsOpts { follow: true, ..Default::default() });
        assert!(spec.args.contains(&"--follow".to_string()));
        assert!(spec.timeout.is_none());
    }

    #[test]
    fn status_default() {
        assert_eq!(OpenClawCli::new().status().args, vec!["status"]);
    }

    #[test]
    fn help_root_and_sub() {
        let cli = OpenClawCli::new();
        assert_eq!(cli.help(None).args, vec!["--help"]);
        assert_eq!(cli.help(Some("update")).args, vec!["update", "--help"]);
    }
}
