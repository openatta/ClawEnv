//! `CommandSpec` — 声明式描述"要执行什么命令"。
//!
//! 纯数据，无 I/O。可被 `ClawCli` 实现返回、可以 Clone、可以日志打印、
//! 可以在单元测试里逐字段断言。真正的执行由 `CommandRunner` 完成。

use std::time::Duration;

/// 一条命令的完整描述。
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// 可执行文件名（`"hermes"`、`"openclaw"`）或绝对路径（测试里指向 fixture）。
    pub binary: String,
    /// 位置参数。
    pub args: Vec<String>,
    /// 附加环境变量（会在子进程 env 上叠加）。
    pub env: Vec<(String, String)>,
    /// 可选 stdin 输入。交互式 CLI 若没有 `--yes` flag，可用此字段注入应答。
    pub stdin: Option<String>,
    /// 可选工作目录。
    pub cwd: Option<String>,
    /// 可选超时。`None` 表示不设上限，依赖 cancel token 控制。
    pub timeout: Option<Duration>,
    /// 输出解析策略。
    pub output_format: OutputFormat,
}

impl CommandSpec {
    /// 构造最常用形态：裸命令 + 参数，Plain 输出，无超时。
    pub fn new(binary: impl Into<String>, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            binary: binary.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: Vec::new(),
            stdin: None,
            cwd: None,
            timeout: None,
            output_format: OutputFormat::Plain,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_output_format(mut self, fmt: OutputFormat) -> Self {
        self.output_format = fmt;
        self
    }

    pub fn with_stdin(mut self, stdin: impl Into<String>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// 拼出"可读"命令行（仅供日志/调试，shell 转义是粗糙的）。
    pub fn display(&self) -> String {
        let mut s = self.binary.clone();
        for a in &self.args {
            s.push(' ');
            if a.contains(' ') || a.contains('"') {
                s.push('"');
                s.push_str(&a.replace('"', "\\\""));
                s.push('"');
            } else {
                s.push_str(a);
            }
        }
        s
    }
}

/// 输出解析策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// 不做结构化解析，原始文本。
    Plain,
    /// stdout 每行是一个独立 JSON 对象（NDJSON / JSON Lines）。
    /// 流式模式下会逐行解析为 `ExecEvent::StructuredProgress`。
    JsonLines,
    /// stdout 整体是一个 JSON。完成后一次性解析到 `ExecResult.structured`。
    JsonFinal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_produces_plain_spec() {
        let spec = CommandSpec::new("hermes", ["status"]);
        assert_eq!(spec.binary, "hermes");
        assert_eq!(spec.args, vec!["status"]);
        assert_eq!(spec.output_format, OutputFormat::Plain);
        assert!(spec.timeout.is_none());
        assert!(spec.stdin.is_none());
    }

    #[test]
    fn builders_chain() {
        let spec = CommandSpec::new("openclaw", ["update", "--json"])
            .with_timeout(Duration::from_secs(120))
            .with_output_format(OutputFormat::JsonFinal)
            .with_env("HERMES_DEBUG", "1")
            .with_cwd("/opt/hermes");
        assert_eq!(spec.timeout, Some(Duration::from_secs(120)));
        assert_eq!(spec.output_format, OutputFormat::JsonFinal);
        assert_eq!(spec.env, vec![("HERMES_DEBUG".into(), "1".into())]);
        assert_eq!(spec.cwd, Some("/opt/hermes".into()));
    }

    #[test]
    fn display_quotes_args_with_spaces() {
        let spec = CommandSpec::new("hermes", ["config", "set", "key", "hello world"]);
        assert_eq!(spec.display(), r#"hermes config set key "hello world""#);
    }
}
