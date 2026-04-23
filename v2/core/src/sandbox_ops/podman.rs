//! Podman-backed SandboxOps. Wraps v2's own `sandbox_backend::PodmanBackend`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::common::{try_exec, CancellationToken, CommandSpec, OpsError, ProgressSink};
use crate::runners::LocalProcessRunner;
use crate::sandbox_backend::{PodmanBackend, SandboxBackend};

use super::ops::SandboxOps;
use super::types::{
    BackendKind, DoctorIssue, PortRule, ResourceStats, SandboxCaps, SandboxDoctorReport,
    SandboxStatus, Severity, VmState,
};

pub struct PodmanOps {
    backend: Arc<dyn SandboxBackend>,
    instance_name: String,
}

impl PodmanOps {
    pub fn new(instance_name: impl Into<String>) -> Self {
        let name = instance_name.into();
        Self {
            backend: Arc::new(PodmanBackend::new(&name)),
            instance_name: name,
        }
    }

    pub fn with_backend(backend: Arc<dyn SandboxBackend>, instance_name: impl Into<String>) -> Self {
        Self { backend, instance_name: instance_name.into() }
    }
}

#[async_trait]
impl SandboxOps for PodmanOps {
    fn backend_kind(&self) -> BackendKind { BackendKind::Podman }
    fn instance_name(&self) -> &str { &self.instance_name }

    fn capabilities(&self) -> SandboxCaps {
        SandboxCaps {
            supports_rename: self.backend.supports_rename(),
            supports_resource_edit: self.backend.supports_resource_edit(),
            supports_port_edit: self.backend.supports_port_edit(),
            supports_snapshot: false,
        }
    }

    async fn status(&self) -> Result<SandboxStatus, OpsError> {
        let state = if self.backend.is_available().await.unwrap_or(false) {
            VmState::Running
        } else {
            VmState::Unknown
        };
        Ok(SandboxStatus {
            backend: BackendKind::Podman,
            instance_name: self.instance_name.clone(),
            state,
            cpu_cores: None,
            memory_mb: None,
            disk_gb: None,
            ip: None,
        })
    }

    async fn start(&self, progress: ProgressSink, _cancel: CancellationToken)
        -> Result<(), OpsError>
    {
        progress.info("sandbox", "podman start").await;
        self.backend.start().await.map_err(OpsError::Other)
    }

    async fn stop(&self, progress: ProgressSink, _cancel: CancellationToken)
        -> Result<(), OpsError>
    {
        progress.info("sandbox", "podman stop").await;
        self.backend.stop().await.map_err(OpsError::Other)
    }

    async fn restart(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>
    {
        self.stop(progress.clone(), cancel.clone()).await?;
        self.start(progress, cancel).await
    }

    async fn list_ports(&self) -> Result<Vec<PortRule>, OpsError> {
        // Read current port bindings from `podman port <container>`.
        // If podman isn't installed or the container doesn't exist / isn't
        // running, return empty (info call — nothing to report is not an error).
        let runner = LocalProcessRunner::new();
        let spec = CommandSpec::new("podman", ["port", &self.instance_name])
            .with_timeout(Duration::from_secs(5));
        let Some(res) = try_exec(&runner, spec, CancellationToken::new()).await?
            else { return Ok(Vec::new()); };
        if !res.success() {
            return Ok(Vec::new());
        }
        Ok(parse_podman_port(&res.stdout))
    }

    async fn add_port(&self, _host: u16, _guest: u16) -> Result<(), OpsError> {
        Err(OpsError::unsupported("add_port",
            "Podman ports are set at container creation; runtime edit not supported"))
    }

    async fn remove_port(&self, _host: u16) -> Result<(), OpsError> {
        Err(OpsError::unsupported("remove_port",
            "Podman ports are set at container creation; runtime edit not supported"))
    }

    async fn doctor(&self) -> Result<SandboxDoctorReport, OpsError> {
        let mut issues = Vec::new();
        match self.backend.is_available().await {
            Ok(false) | Err(_) => issues.push(DoctorIssue {
                id: "vm-not-running".into(),
                severity: Severity::Error,
                message: "Podman container is not running".into(),
                repair_hint: Some("clawops sandbox start".into()),
                auto_repairable: true,
            }),
            Ok(true) => {}
        }
        Ok(SandboxDoctorReport {
            backend: BackendKind::Podman,
            instance_name: self.instance_name.clone(),
            issues,
            checked_at: Utc::now().to_rfc3339(),
        })
    }

    async fn repair(&self, _issue_ids: &[String], _progress: ProgressSink)
        -> Result<(), OpsError>
    {
        Err(OpsError::unsupported("repair",
            "Podman repair actions — planned in Stage B"))
    }

    async fn stats(&self) -> Result<ResourceStats, OpsError> {
        let s = self.backend.stats().await.map_err(OpsError::Other)?;
        Ok(ResourceStats {
            cpu_percent: s.cpu_percent,
            memory_used_mb: s.memory_used_mb,
            memory_limit_mb: s.memory_limit_mb,
        })
    }

    async fn dump_logs(&self, _tail: Option<u32>) -> Result<String, OpsError> {
        Err(OpsError::unsupported("dump_logs",
            "Podman log dump — planned in Stage B"))
    }
}

/// Parse `podman port <container>` output into PortRules.
///
/// Format: each line is `<guest_port>/<proto> -> <host_addr>:<host_port>`.
/// Example:
/// ```text
/// 3000/tcp -> 0.0.0.0:3000
/// 8080/tcp -> 0.0.0.0:8080
/// ```
pub(crate) fn parse_podman_port(out: &str) -> Vec<PortRule> {
    let mut rules = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let (left, right) = match line.split_once("->") {
            Some(t) => t,
            None => continue,
        };
        // Left: "3000/tcp" → "3000"
        let guest_str = left.trim().split('/').next().unwrap_or("").trim();
        // Right: "0.0.0.0:3000" → "3000"
        let host_str = right.trim().rsplit(':').next().unwrap_or("").trim();
        let guest = match guest_str.parse::<u16>() { Ok(v) => v, Err(_) => continue };
        let host = match host_str.parse::<u16>() { Ok(v) => v, Err(_) => continue };
        rules.push(PortRule {
            host, guest,
            native_id: Some(line.to_string()),
        });
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind() {
        assert_eq!(PodmanOps::new("x").backend_kind(), BackendKind::Podman);
    }

    #[test]
    fn parse_podman_port_happy_path() {
        let out = "3000/tcp -> 0.0.0.0:3000\n8080/tcp -> 0.0.0.0:8080\n";
        let rules = parse_podman_port(out);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].host, 3000);
        assert_eq!(rules[0].guest, 3000);
        assert_eq!(rules[1].host, 8080);
    }

    #[test]
    fn parse_podman_port_empty() {
        assert!(parse_podman_port("").is_empty());
    }

    #[test]
    fn parse_podman_port_ignores_garbage_lines() {
        let out = "not-a-rule\n3000/tcp -> 0.0.0.0:3000\ngarbage garbage\n";
        let rules = parse_podman_port(out);
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn parse_podman_port_handles_ipv6_addr() {
        // Uses rsplit(':') so IPv6 host addrs still work.
        let out = "3000/tcp -> [::]:3000\n";
        let rules = parse_podman_port(out);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].host, 3000);
    }
}
