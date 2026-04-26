//! Hermes Agent CLI 映射。
//!
//! 基于 `hermes-agent.nousresearch.com/docs/reference/cli-commands` 的官方参考。
//! 核实的子命令：update / doctor / config / logs / status / version / help ...

use std::time::Duration;

use async_trait::async_trait;

use crate::claw_ops::claw_cli::{ClawCli, DoctorOpts, LogsOpts, UpdateOpts};
use crate::claw_ops::command::{CommandSpec, OutputFormat};

/// 各类命令的默认超时。单位秒。
/// Hermes update 涉及 git pull + uv pip install，官方文档提到依赖下载可能较慢。
const UPDATE_TIMEOUT_SECS: u64 = 15 * 60;   // 15 min
const DOCTOR_TIMEOUT_SECS: u64 = 60;
const CONFIG_TIMEOUT_SECS: u64 = 10;
const STATUS_TIMEOUT_SECS: u64 = 10;
const VERSION_TIMEOUT_SECS: u64 = 5;
const HELP_TIMEOUT_SECS: u64 = 5;
const LOGS_TIMEOUT_SECS: u64 = 30;  // 不带 --follow 时的上限

#[derive(Default)]
pub struct HermesCli;

impl HermesCli {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl ClawCli for HermesCli {
    fn id(&self) -> &str { "hermes" }
    fn binary(&self) -> &str { "hermes" }
    fn supports_native(&self) -> bool { false }  // 官方明确 "Native Windows is not supported"；ClawEnv 中 Hermes 仅沙盒

    fn update(&self, opts: UpdateOpts) -> CommandSpec {
        let mut args = vec!["update".to_string()];
        if opts.non_interactive {
            args.push("--yes".into());
        }
        // opts.json / dry_run / channel / tag / no_restart 在 Hermes 里目前无原生对应，
        // 按"忽略多余选项"的温和策略处理 —— 保证调用方可以用统一 UpdateOpts 传参。
        CommandSpec::new(self.binary(), args)
            .with_timeout(Duration::from_secs(UPDATE_TIMEOUT_SECS))
            .with_output_format(OutputFormat::Plain)
    }

    fn doctor(&self, opts: DoctorOpts) -> CommandSpec {
        let mut args = vec!["doctor".to_string()];
        if opts.fix {
            args.push("--fix".into());
        }
        // doctor 目前没有 --json 官方支持，输出按 Plain 处理；若上游后续加上，
        // 我们再把 output_format 条件切到 JsonFinal 即可。
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
        // follow 模式走无限流，不设超时 —— 由 cancel token 负责结束
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
        let cli = HermesCli::new();
        assert_eq!(cli.id(), "hermes");
        assert_eq!(cli.binary(), "hermes");
        assert!(!cli.supports_native());
    }

    #[test]
    fn update_default_opts_is_interactive() {
        let spec = HermesCli::new().update(UpdateOpts::default());
        assert_eq!(spec.binary, "hermes");
        assert_eq!(spec.args, vec!["update"]);
        assert_eq!(spec.timeout, Some(Duration::from_secs(UPDATE_TIMEOUT_SECS)));
    }

    #[test]
    fn update_non_interactive_adds_yes() {
        let spec = HermesCli::new().update(UpdateOpts { non_interactive: true, ..Default::default() });
        assert_eq!(spec.args, vec!["update", "--yes"]);
    }

    #[test]
    fn update_ignores_openclaw_only_opts() {
        // 传了 OpenClaw 才有的选项，Hermes 实现应优雅忽略（保持 args 干净）
        let spec = HermesCli::new().update(UpdateOpts {
            json: true, dry_run: true, channel: Some("beta".into()),
            tag: Some("v1.0".into()), no_restart: true,
            ..Default::default()
        });
        assert_eq!(spec.args, vec!["update"], "should not include OpenClaw-specific flags");
    }

    #[test]
    fn doctor_default_no_fix() {
        let spec = HermesCli::new().doctor(DoctorOpts::default());
        assert_eq!(spec.args, vec!["doctor"]);
    }

    #[test]
    fn doctor_with_fix() {
        let spec = HermesCli::new().doctor(DoctorOpts { fix: true, ..Default::default() });
        assert_eq!(spec.args, vec!["doctor", "--fix"]);
    }

    #[test]
    fn version_command() {
        let spec = HermesCli::new().version();
        assert_eq!(spec.args, vec!["version"]);
    }

    #[test]
    fn config_get_preserves_key() {
        let spec = HermesCli::new().config_get("model.default");
        assert_eq!(spec.args, vec!["config", "get", "model.default"]);
    }

    #[test]
    fn config_set_preserves_key_and_value() {
        let spec = HermesCli::new().config_set("model.default", "anthropic/claude-sonnet");
        assert_eq!(spec.args, vec!["config", "set", "model.default", "anthropic/claude-sonnet"]);
    }

    #[test]
    fn config_list() {
        let spec = HermesCli::new().config_list();
        assert_eq!(spec.args, vec!["config", "list"]);
    }

    #[test]
    fn logs_with_tail_and_level() {
        let spec = HermesCli::new().logs(LogsOpts {
            tail: Some(100),
            follow: false,
            level: Some("error".into()),
        });
        assert_eq!(spec.args, vec!["logs", "--tail", "100", "--level", "error"]);
        assert!(spec.timeout.is_some(), "non-follow logs should have timeout");
    }

    #[test]
    fn logs_follow_has_no_timeout() {
        let spec = HermesCli::new().logs(LogsOpts { follow: true, ..Default::default() });
        assert!(spec.args.contains(&"--follow".to_string()));
        assert!(spec.timeout.is_none(), "follow mode must rely on cancel token");
    }

    #[test]
    fn status_command() {
        let spec = HermesCli::new().status();
        assert_eq!(spec.args, vec!["status"]);
    }

    #[test]
    fn help_root_and_sub() {
        let cli = HermesCli::new();
        assert_eq!(cli.help(None).args, vec!["--help"]);
        assert_eq!(cli.help(Some("update")).args, vec!["update", "--help"]);
    }
}
