//! Lima backend — full VM lifecycle via `limactl`.

use std::time::Duration;

use async_trait::async_trait;

use crate::common::{try_exec, CancellationToken, CommandRunner, CommandSpec};
use crate::paths::{lima_home, v2_templates_dir};
use crate::provisioning::{render_lima_yaml, CreateOpts};
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

    async fn is_present(&self) -> anyhow::Result<bool> {
        // Lima stores every VM as a subdir of LIMA_HOME — presence of
        // <lima_home>/<instance>/lima.yaml means "defined" (stopped or running).
        Ok(self.instance_dir().join("lima.yaml").exists())
    }

    async fn create(&self, opts: &CreateOpts) -> anyhow::Result<()> {
        // Idempotency: a pre-existing instance directory means Lima has
        // already provisioned this VM. Treat create() as a no-op so
        // callers can use it as a "make sure it exists" guarantee.
        if self.is_present().await.unwrap_or(false) {
            return Ok(());
        }

        // Stage the rendered YAML. Lima wants a file path, not stdin.
        let tpl_dir = v2_templates_dir();
        tokio::fs::create_dir_all(&tpl_dir).await
            .map_err(|e| anyhow::anyhow!("create template dir {}: {e}", tpl_dir.display()))?;
        let tpl_path = tpl_dir.join(format!("{}.yaml", self.instance));
        let yaml = render_lima_yaml(opts);
        tokio::fs::write(&tpl_path, &yaml).await
            .map_err(|e| anyhow::anyhow!("write template {}: {e}", tpl_path.display()))?;

        // Ensure the host workspace dir exists — Lima's `mounts:` entry
        // for it will fail the start otherwise.
        if let Err(e) = tokio::fs::create_dir_all(&opts.workspace_dir).await {
            anyhow::bail!(
                "create workspace dir {}: {e}",
                opts.workspace_dir.display()
            );
        }

        // `limactl start --name <inst> --tty=false <path>` — blocks
        // until cloud-init finishes (can take 5–10 min on first boot
        // when packages are fetched). We give it a generous 20 min
        // budget; callers that need finer-grained feedback should
        // drive this via ProgressSink (deferred until we introduce a
        // streaming variant).
        let spec = CommandSpec::new(
            "limactl",
            [
                "start",
                "--name",
                self.instance.as_str(),
                "--tty=false",
                tpl_path.to_str().ok_or_else(|| {
                    anyhow::anyhow!("non-UTF8 template path: {}", tpl_path.display())
                })?,
            ],
        )
        .with_timeout(Duration::from_secs(20 * 60));

        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!(
                "limactl start --name {} --tty=false {}: exit {}\nstderr:\n{}",
                self.instance,
                tpl_path.display(),
                res.exit_code,
                tail_n(&res.stderr, 40),
            );
        }
        Ok(())
    }

    async fn destroy(&self) -> anyhow::Result<()> {
        // No-op when the instance isn't present at all — idempotent.
        if !self.is_present().await.unwrap_or(false) {
            return Ok(());
        }
        // `limactl delete --force <name>` tolerates running VMs by
        // stopping first. Kills the VM dir under LIMA_HOME.
        let spec = CommandSpec::new("limactl", ["delete", "--force", &self.instance])
            .with_timeout(Duration::from_secs(2 * 60));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!(
                "limactl delete --force {}: exit {}\nstderr:\n{}",
                self.instance,
                res.exit_code,
                tail_n(&res.stderr, 20),
            );
        }
        // Clean up our staged template — best-effort.
        let tpl_path = v2_templates_dir().join(format!("{}.yaml", self.instance));
        let _ = tokio::fs::remove_file(&tpl_path).await;
        Ok(())
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

/// Last N lines of a string — used for error-message tails so we don't
/// dump multi-MB stderr into user-facing messages.
fn tail_n(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= n {
        s.to_string()
    } else {
        lines[lines.len() - n..].join("\n")
    }
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
    fn tail_n_shorter_than_requested_returns_all() {
        assert_eq!(tail_n("a\nb", 10), "a\nb");
    }

    #[test]
    fn tail_n_trims_to_last_lines() {
        assert_eq!(tail_n("a\nb\nc\nd", 2), "c\nd");
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
