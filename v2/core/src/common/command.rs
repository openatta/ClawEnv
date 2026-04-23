//! Declarative "what to run" — no I/O. CommandRunner turns this into bytes on the wire.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub binary: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<String>,
    pub cwd: Option<String>,
    pub timeout: Option<Duration>,
    pub output_format: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Plain,
    JsonLines,
    JsonFinal,
}

impl CommandSpec {
    pub fn new(
        binary: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
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

    pub fn with_timeout(mut self, t: Duration) -> Self { self.timeout = Some(t); self }
    pub fn with_output_format(mut self, f: OutputFormat) -> Self { self.output_format = f; self }
    pub fn with_stdin(mut self, s: impl Into<String>) -> Self { self.stdin = Some(s.into()); self }
    pub fn with_env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }
    pub fn with_cwd(mut self, c: impl Into<String>) -> Self { self.cwd = Some(c.into()); self }

    /// Human-readable command line for logs. Shell-escape is coarse; not intended
    /// to be eval'd.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let s = CommandSpec::new("hermes", ["status"]);
        assert_eq!(s.output_format, OutputFormat::Plain);
        assert!(s.timeout.is_none());
        assert!(s.stdin.is_none());
        assert!(s.env.is_empty());
    }

    #[test]
    fn builders_chain() {
        let s = CommandSpec::new("openclaw", ["update", "--json"])
            .with_timeout(Duration::from_secs(60))
            .with_output_format(OutputFormat::JsonFinal)
            .with_env("LOG", "1")
            .with_cwd("/tmp");
        assert_eq!(s.timeout, Some(Duration::from_secs(60)));
        assert_eq!(s.output_format, OutputFormat::JsonFinal);
        assert_eq!(s.env, vec![("LOG".into(), "1".into())]);
        assert_eq!(s.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn display_quotes_spaces() {
        let s = CommandSpec::new("hermes", ["config", "set", "k", "v w"]);
        assert_eq!(s.display(), r#"hermes config set k "v w""#);
    }
}
