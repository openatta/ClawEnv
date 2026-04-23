//! Lima backend — execute commands via `limactl shell <instance>`.
//!
//! Scope: runtime ops only. Creation/destruction is out of scope for v2
//! (see sandbox_backend/mod.rs for rationale).

use std::time::Duration;

use async_trait::async_trait;

use crate::common::{try_exec, CancellationToken, CommandRunner, CommandSpec};
use crate::paths::lima_home;
use crate::runners::LocalProcessRunner;

use super::{ResourceStats, SandboxBackend};

pub struct LimaBackend {
    instance: String,
    runner: LocalProcessRunner,
}

impl LimaBackend {
    pub fn new(instance: impl Into<String>) -> Self {
        Self {
            instance: instance.into(),
            runner: LocalProcessRunner::new(),
        }
    }

    fn instance_dir(&self) -> std::path::PathBuf {
        lima_home().join(&self.instance)
    }
}

#[async_trait]
impl SandboxBackend for LimaBackend {
    fn name(&self) -> &str { "Lima" }
    fn instance(&self) -> &str { &self.instance }

    async fn is_available(&self) -> anyhow::Result<bool> {
        if !self.instance_dir().exists() {
            return Ok(false);
        }
        // `limactl list <instance> --format json` returns the row — parse state.
        // Tolerate `limactl` not being installed — report "not available".
        let spec = CommandSpec::new("limactl", ["list", &self.instance, "--format", "json"])
            .with_timeout(Duration::from_secs(5));
        let Some(res) = try_exec(&self.runner, spec, CancellationToken::new()).await?
            else { return Ok(false); };
        if !res.success() { return Ok(false); }
        let first = res.stdout.lines().next().unwrap_or("").trim();
        if first.is_empty() { return Ok(false); }
        let v: serde_json::Value = match serde_json::from_str(first) {
            Ok(v) => v,
            Err(_) => return Ok(false),
        };
        Ok(v["status"].as_str() == Some("Running"))
    }

    async fn start(&self) -> anyhow::Result<()> {
        let spec = CommandSpec::new("limactl", ["start", &self.instance])
            .with_timeout(Duration::from_secs(5 * 60));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("limactl start {} failed (exit {}): {}",
                self.instance, res.exit_code, res.stderr);
        }
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        let spec = CommandSpec::new("limactl", ["stop", &self.instance])
            .with_timeout(Duration::from_secs(2 * 60));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("limactl stop {} failed (exit {}): {}",
                self.instance, res.exit_code, res.stderr);
        }
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> anyhow::Result<String> {
        let spec = CommandSpec::new("limactl", ["shell", &self.instance, "sh", "-c", cmd])
            .with_timeout(Duration::from_secs(5 * 60));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("limactl exec failed (exit {}): stderr: {}",
                res.exit_code, res.stderr);
        }
        Ok(res.stdout)
    }

    async fn stats(&self) -> anyhow::Result<ResourceStats> {
        // Lima's `limactl info` dumps machine-wide JSON. Per-instance stats
        // aren't exposed uniformly in 2.x; we return zeros for now.
        // Stage D can query `vmType`/cgroup-specific metrics.
        Ok(ResourceStats::default())
    }

    async fn edit_port_forwards(&self, forwards: &[(u16, u16)]) -> anyhow::Result<()> {
        // Rewrite the portForwards: block in lima.yaml.
        let yaml_path = self.instance_dir().join("lima.yaml");
        if !yaml_path.exists() {
            anyhow::bail!("lima.yaml not found for instance {}: {}",
                self.instance, yaml_path.display());
        }
        let content = tokio::fs::read_to_string(&yaml_path).await?;
        let new = rewrite_port_forwards(&content, forwards);
        tokio::fs::write(&yaml_path, new).await?;

        // Lima re-reads the YAML on next start; a running VM needs a restart
        // to pick up port forward changes. Restart it (if running) so the
        // user sees effect immediately.
        if self.is_available().await.unwrap_or(false) {
            let _ = self.stop().await;
            self.start().await?;
        }
        Ok(())
    }

    fn supports_rename(&self) -> bool { true }
    fn supports_resource_edit(&self) -> bool { true }
    fn supports_port_edit(&self) -> bool { true }
}

/// Rewrite the `portForwards:` block of a lima.yaml with the given set
/// (or append one if the key isn't present).
pub(crate) fn rewrite_port_forwards(yaml: &str, forwards: &[(u16, u16)]) -> String {
    let mut out = String::new();
    let mut in_block = false;
    let mut emitted_new = false;

    for raw in yaml.lines() {
        let line = raw;
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        if !in_block {
            if indent == 0 && trimmed.starts_with("portForwards:") {
                in_block = true;
                // Emit our new block header + entries.
                out.push_str("portForwards:\n");
                for (host, guest) in forwards {
                    out.push_str(&format!("  - guestPort: {guest}\n    hostPort: {host}\n"));
                }
                emitted_new = true;
                continue;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }

        // In block: skip lines until dedent or new top-level key.
        if indent == 0 && !trimmed.starts_with('-') && !trimmed.is_empty() {
            in_block = false;
            out.push_str(line);
            out.push('\n');
        }
        // else: drop the old portForward entries
    }

    if !emitted_new {
        // File didn't contain portForwards: — append at EOF.
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("portForwards:\n");
        for (host, guest) in forwards {
            out.push_str(&format!("  - guestPort: {guest}\n    hostPort: {host}\n"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_and_name() {
        let b = LimaBackend::new("demo");
        assert_eq!(b.name(), "Lima");
        assert_eq!(b.instance(), "demo");
    }

    #[test]
    fn rewrite_port_forwards_replaces_existing_block() {
        let input = "cpus: 4\nportForwards:\n  - guestPort: 22\n    hostPort: 60022\nmemory: 8GiB\n";
        let rewritten = rewrite_port_forwards(input, &[(3000, 3000), (8080, 8080)]);
        assert!(rewritten.contains("cpus: 4"));
        assert!(rewritten.contains("memory: 8GiB"));
        assert!(rewritten.contains("guestPort: 3000"));
        assert!(rewritten.contains("hostPort: 3000"));
        assert!(!rewritten.contains("60022"),
            "old rule should be gone\n{rewritten}");
    }

    #[test]
    fn rewrite_port_forwards_appends_when_missing() {
        let input = "cpus: 4\nmemory: 8GiB\n";
        let rewritten = rewrite_port_forwards(input, &[(3000, 3000)]);
        assert!(rewritten.contains("portForwards:"));
        assert!(rewritten.contains("guestPort: 3000"));
    }

    #[test]
    fn rewrite_port_forwards_empty_list_drops_block() {
        let input = "cpus: 4\nportForwards:\n  - guestPort: 22\n    hostPort: 60022\nmemory: 8GiB\n";
        let rewritten = rewrite_port_forwards(input, &[]);
        // portForwards: header is still emitted but with zero entries.
        assert!(rewritten.contains("portForwards:"));
        assert!(!rewritten.contains("guestPort:"),
            "no guestPort lines remain:\n{rewritten}");
    }
}
