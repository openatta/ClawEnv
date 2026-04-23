//! `ClawCli` trait —— 每个 Claw 产品把自己的 CLI 子命令映射成 `CommandSpec`。
//!
//! 纯声明式：方法返回 `CommandSpec`，不执行。真正的执行由 `CommandRunner` 完成。
//! 这种形态让我们可以：
//! - 对每个方法写单元测试（断言 args 等字段），无需 VM 无需真实 binary
//! - dry-run（打印命令给用户看）
//! - 审计（日志里记录每次执行什么命令）

use async_trait::async_trait;

use super::command::CommandSpec;

/// `update` 子命令的选项集合。各 Claw 实现自己把这些映射到各自的 flag。
#[derive(Debug, Clone, Default)]
pub struct UpdateOpts {
    /// 非交互模式。Hermes 目前对应 `--yes`；OpenClaw 对应 `--yes`。
    pub non_interactive: bool,
    /// 请求结构化 JSON 输出。OpenClaw `--json` 原生支持；Hermes 若无则实现里退化为 Plain。
    pub json: bool,
    /// 预演模式。OpenClaw `--dry-run`；Hermes 目前无原生对应。
    pub dry_run: bool,
    /// 升级通道（OpenClaw：stable/beta/dev）。Hermes 忽略此字段。
    pub channel: Option<String>,
    /// 指定版本/tag（OpenClaw `--tag`）。Hermes 忽略。
    pub tag: Option<String>,
    /// 升级期间不重启 gateway（OpenClaw `--no-restart`）。Hermes 忽略。
    pub no_restart: bool,
}

/// `doctor` 子命令的选项集合。
#[derive(Debug, Clone, Default)]
pub struct DoctorOpts {
    /// 尝试自动修复（Hermes `--fix`；OpenClaw 暂无原生对应，实现里忽略）。
    pub fix: bool,
    /// JSON 输出（若支持）。
    pub json: bool,
}

/// `logs` 子命令的选项集合。
#[derive(Debug, Clone, Default)]
pub struct LogsOpts {
    /// 只看最后 N 行。
    pub tail: Option<u32>,
    /// 持续跟随（tail -f）。
    pub follow: bool,
    /// 过滤等级（"debug" / "info" / "warn" / "error"）。
    pub level: Option<String>,
}

#[async_trait]
pub trait ClawCli: Send + Sync {
    /// 产品 ID：`"hermes"` / `"openclaw"`。
    fn id(&self) -> &str;
    /// 可执行文件名（通常等于 id）。
    fn binary(&self) -> &str;
    /// 该 Claw 是否支持 Native（非沙盒）部署。Hermes = false。
    fn supports_native(&self) -> bool;

    // ——— 生命周期管理 ———
    fn update(&self, opts: UpdateOpts) -> CommandSpec;
    fn doctor(&self, opts: DoctorOpts) -> CommandSpec;
    fn version(&self) -> CommandSpec;

    // ——— 配置 ———
    fn config_get(&self, key: &str) -> CommandSpec;
    fn config_set(&self, key: &str, value: &str) -> CommandSpec;
    fn config_list(&self) -> CommandSpec;

    // ——— 调测 / 监控 ———
    fn logs(&self, opts: LogsOpts) -> CommandSpec;
    fn status(&self) -> CommandSpec;

    // ——— 元命令 ———
    fn help(&self, subcommand: Option<&str>) -> CommandSpec;
}
