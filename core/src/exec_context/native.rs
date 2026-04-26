//! `ExecutionContext` impl that runs commands on the host process
//! tree, scoped to an instance's native prefix dir
//! (`~/.clawenv/native/<inst>`).
//!
//! Unlike `SandboxContext`, exec here is direct — no `limactl shell`
//! or `wsl --exec` indirection. The trade-off: the host's PATH leaks
//! in unless we explicitly scope. We prepend `<prefix>/bin:<prefix>/
//! node/bin:<prefix>/git/bin` so the claw's own toolchain wins.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::{ContextKind, ExecError, ExecutionContext};

pub struct NativeContext {
    prefix: PathBuf,
}

impl NativeContext {
    pub fn new(prefix: PathBuf) -> Self {
        Self { prefix }
    }

    /// PATH the spawned process gets: clawenv's own bin dirs first,
    /// then the system PATH so users still get `git` / `node` / etc.
    /// when v2 hasn't bootstrapped them.
    fn scoped_path(&self) -> String {
        let mut entries: Vec<String> = Vec::new();
        for sub in ["bin", "node/bin", "git/bin"] {
            let p = self.prefix.join(sub);
            if p.exists() {
                entries.push(p.to_string_lossy().into_owned());
            }
        }
        if let Ok(sys) = std::env::var("PATH") {
            entries.push(sys);
        }
        entries.join(":")
    }
}

#[async_trait]
impl ExecutionContext for NativeContext {
    fn id(&self) -> String {
        format!("native:{}", self.prefix.display())
    }

    fn kind(&self) -> ContextKind {
        ContextKind::Native { prefix: self.prefix.clone() }
    }

    async fn exec(&self, argv: &[&str]) -> Result<String, ExecError> {
        if argv.is_empty() {
            return Err(ExecError::Other("empty argv".into()));
        }
        let mut cmd = tokio::process::Command::new(argv[0]);
        cmd.args(&argv[1..]);
        cmd.env("PATH", self.scoped_path());
        cmd.kill_on_drop(true);
        let out = cmd.output().await.map_err(ExecError::Io)?;
        if !out.status.success() {
            let code = out.status.code().unwrap_or(-1);
            let stderr_tail = String::from_utf8_lossy(&out.stderr)
                .lines().rev().take(20).collect::<Vec<_>>()
                .into_iter().rev().collect::<Vec<_>>().join("\n");
            return Err(ExecError::NonZero { code, stderr_tail });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    async fn is_alive(&self) -> bool {
        // Host is always "alive". The more useful question — is the
        // claw binary present at <prefix>/bin/<binary> — depends on
        // which claw, so callers ask that explicitly via `exec`.
        true
    }

    fn resolve_to_host(&self, ctx_path: &Path) -> Option<PathBuf> {
        // Native is on-host: every path is a host path.
        Some(ctx_path.to_path_buf())
    }

    async fn exec_streaming(
        &self,
        argv: &[&str],
        on_line: &mut (dyn FnMut(String) + Send),
    ) -> Result<i32, ExecError> {
        use std::process::Stdio;
        use tokio::io::{AsyncBufReadExt, BufReader};

        if argv.is_empty() {
            return Err(ExecError::Other("empty argv".into()));
        }
        let mut cmd = tokio::process::Command::new(argv[0]);
        cmd.args(&argv[1..]);
        cmd.env("PATH", self.scoped_path());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().map_err(ExecError::Io)?;
        let stdout = child.stdout.take()
            .ok_or_else(|| ExecError::Other("no stdout".into()))?;
        let mut reader = BufReader::new(stdout).lines();
        while let Some(line) = reader.next_line().await.map_err(ExecError::Io)? {
            on_line(line);
        }
        let status = child.wait().await.map_err(ExecError::Io)?;
        Ok(status.code().unwrap_or(-1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn exec_runs_against_host() {
        let tmp = TempDir::new().unwrap();
        let c = NativeContext::new(tmp.path().to_path_buf());
        let out = c.exec(&["echo", "hello"]).await.unwrap();
        assert_eq!(out.trim(), "hello");
    }

    #[tokio::test]
    async fn exec_nonzero_carries_code() {
        let tmp = TempDir::new().unwrap();
        let c = NativeContext::new(tmp.path().to_path_buf());
        let err = c.exec(&["sh", "-c", "exit 42"]).await.unwrap_err();
        match err {
            ExecError::NonZero { code, .. } => assert_eq!(code, 42),
            other => panic!("expected NonZero, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_to_host_is_passthrough() {
        let tmp = TempDir::new().unwrap();
        let c = NativeContext::new(tmp.path().to_path_buf());
        let p = Path::new("/etc/hostname");
        assert_eq!(c.resolve_to_host(p), Some(p.to_path_buf()));
    }

    #[tokio::test]
    async fn streaming_collects_lines() {
        let tmp = TempDir::new().unwrap();
        let c = NativeContext::new(tmp.path().to_path_buf());
        let mut collected: Vec<String> = Vec::new();
        let mut on_line = |line: String| {
            collected.push(line);
        };
        let exit = c.exec_streaming(
            &["sh", "-c", "echo a; echo b; echo c"],
            &mut on_line,
        ).await.unwrap();
        assert_eq!(exit, 0);
        assert_eq!(collected, vec!["a", "b", "c"]);
    }
}
