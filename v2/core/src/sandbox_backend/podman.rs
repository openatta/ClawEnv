//! Podman backend — execute via `podman exec <container>`.
//!
//! Container ports are baked in at `podman run` time; runtime port-edit is
//! not supported (matches v2 SandboxOps behavior).

use std::time::Duration;

use async_trait::async_trait;

use crate::common::{try_exec, CancellationToken, CommandRunner, CommandSpec};
use crate::runners::LocalProcessRunner;

use super::{ResourceStats, SandboxBackend};

pub struct PodmanBackend {
    instance: String,
    runner: LocalProcessRunner,
}

impl PodmanBackend {
    pub fn new(instance: impl Into<String>) -> Self {
        Self {
            instance: instance.into(),
            runner: LocalProcessRunner::new(),
        }
    }
}

#[async_trait]
impl SandboxBackend for PodmanBackend {
    fn name(&self) -> &str { "Podman" }
    fn instance(&self) -> &str { &self.instance }

    async fn is_available(&self) -> anyhow::Result<bool> {
        // Tolerate `podman` not being installed — report "not available".
        let spec = CommandSpec::new("podman",
            ["container", "inspect", &self.instance, "--format", "{{.State.Status}}"])
            .with_timeout(Duration::from_secs(5));
        let Some(res) = try_exec(&self.runner, spec, CancellationToken::new()).await?
            else { return Ok(false); };
        if !res.success() { return Ok(false); }
        Ok(res.stdout.trim() == "running")
    }

    async fn start(&self) -> anyhow::Result<()> {
        let spec = CommandSpec::new("podman", ["start", &self.instance])
            .with_timeout(Duration::from_secs(30));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("podman start {} failed: {}", self.instance, res.stderr);
        }
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        let spec = CommandSpec::new("podman", ["stop", &self.instance])
            .with_timeout(Duration::from_secs(30));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("podman stop {} failed: {}", self.instance, res.stderr);
        }
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> anyhow::Result<String> {
        let spec = CommandSpec::new("podman",
            ["exec", &self.instance, "sh", "-c", cmd])
            .with_timeout(Duration::from_secs(5 * 60));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("podman exec failed (exit {}): {}", res.exit_code, res.stderr);
        }
        Ok(res.stdout)
    }

    async fn stats(&self) -> anyhow::Result<ResourceStats> {
        // `podman stats --no-stream --format json <name>`. Tolerates podman-missing.
        let spec = CommandSpec::new("podman",
            ["stats", "--no-stream", "--format", "json", &self.instance])
            .with_timeout(Duration::from_secs(10));
        let Some(res) = try_exec(&self.runner, spec, CancellationToken::new()).await?
            else { return Ok(ResourceStats::default()); };
        if !res.success() {
            return Ok(ResourceStats::default());
        }
        let trimmed = res.stdout.trim();
        if trimmed.is_empty() { return Ok(ResourceStats::default()); }
        let v: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => return Ok(ResourceStats::default()),
        };
        // podman stats returns an array of objects.
        let row = v.as_array().and_then(|a| a.first()).cloned().unwrap_or(v);
        let cpu_percent = parse_cpu_percent(
            row["cpu_percent"].as_str().unwrap_or("")
        );
        let memory_used_mb = row["mem_usage"].as_u64().map(|b| b / 1024 / 1024).unwrap_or(0);
        let memory_limit_mb = row["mem_limit"].as_u64().map(|b| b / 1024 / 1024).unwrap_or(0);
        Ok(ResourceStats { cpu_percent, memory_used_mb, memory_limit_mb })
    }

    async fn edit_port_forwards(&self, _forwards: &[(u16, u16)]) -> anyhow::Result<()> {
        anyhow::bail!("Podman ports are set at container creation; edit not supported")
    }

    fn supports_rename(&self) -> bool { false }
    fn supports_resource_edit(&self) -> bool { false }
    fn supports_port_edit(&self) -> bool { false }
}

fn parse_cpu_percent(s: &str) -> f32 {
    // Format: "12.34%" (new) or "0.05" (old). Strip "%" if present.
    s.trim().trim_end_matches('%').parse::<f32>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_and_name() {
        let b = PodmanBackend::new("demo");
        assert_eq!(b.name(), "Podman");
        assert_eq!(b.instance(), "demo");
    }

    #[test]
    fn parse_cpu_variants() {
        assert!((parse_cpu_percent("12.5%") - 12.5).abs() < 0.01);
        assert!((parse_cpu_percent("0.05") - 0.05).abs() < 0.001);
        assert_eq!(parse_cpu_percent("garbage"), 0.0);
    }
}
