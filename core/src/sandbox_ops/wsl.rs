//! WSL2-backed SandboxOps. Wraps v2's own `sandbox_backend::WslBackend`.

use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::common::{CancellationToken, OpsError, ProgressSink};
#[cfg(target_os = "windows")]
use crate::common::{CommandRunner, CommandSpec};
#[cfg(target_os = "windows")]
use crate::runners::LocalProcessRunner;
use crate::sandbox_backend::{SandboxBackend, WslBackend};

use super::ops::SandboxOps;
use super::types::{
    BackendKind, DoctorIssue, PortRule, ResourceStats, SandboxCaps, SandboxDoctorReport,
    SandboxStatus, Severity, VmState,
};

pub struct WslOps {
    backend: Arc<dyn SandboxBackend>,
    instance_name: String,
}

impl WslOps {
    pub fn new(instance_name: impl Into<String>) -> Self {
        let name = instance_name.into();
        Self {
            backend: Arc::new(WslBackend::new(&name)),
            instance_name: name,
        }
    }

    pub fn with_backend(backend: Arc<dyn SandboxBackend>, instance_name: impl Into<String>) -> Self {
        Self { backend, instance_name: instance_name.into() }
    }
}

#[async_trait]
impl SandboxOps for WslOps {
    fn backend_kind(&self) -> BackendKind { BackendKind::Wsl2 }
    fn instance_name(&self) -> &str { &self.instance_name }

    fn capabilities(&self) -> SandboxCaps {
        SandboxCaps {
            supports_rename: self.backend.supports_rename(),
            supports_resource_edit: self.backend.supports_resource_edit(),
            supports_port_edit: self.backend.supports_port_edit(),
            supports_snapshot: self.backend.supports_snapshot(),
        }
    }

    async fn status(&self) -> Result<SandboxStatus, OpsError> {
        let state = if self.backend.is_available().await.unwrap_or(false) {
            VmState::Running
        } else {
            VmState::Unknown
        };
        Ok(SandboxStatus {
            backend: BackendKind::Wsl2,
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
        progress.info("sandbox", "wsl start").await;
        self.backend.start().await.map_err(OpsError::Other)
    }

    async fn stop(&self, progress: ProgressSink, _cancel: CancellationToken)
        -> Result<(), OpsError>
    {
        progress.info("sandbox", "wsl stop").await;
        self.backend.stop().await.map_err(OpsError::Other)
    }

    async fn restart(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>
    {
        self.stop(progress.clone(), cancel.clone()).await?;
        self.start(progress, cancel).await
    }

    async fn list_ports(&self) -> Result<Vec<PortRule>, OpsError> {
        // netsh is Windows-only. On non-Windows, return empty (not an error —
        // caller may be on mac/linux running `clawops sandbox port list --backend wsl2`
        // just out of curiosity; there's nothing to list).
        #[cfg(not(target_os = "windows"))]
        { return Ok(Vec::new()); }

        #[cfg(target_os = "windows")]
        {
            let runner = LocalProcessRunner::new();
            let spec = CommandSpec::new("netsh", ["interface", "portproxy", "show", "v4tov4"])
                .with_timeout(Duration::from_secs(5));
            let res = runner.exec(spec, CancellationToken::new()).await?;
            if !res.success() {
                return Ok(Vec::new());
            }
            Ok(parse_netsh_portproxy(&res.stdout))
        }
    }

    async fn add_port(&self, host: u16, guest: u16) -> Result<(), OpsError> {
        if !self.backend.supports_port_edit() {
            return Err(OpsError::unsupported("add_port",
                "WSL port edit unavailable on this host"));
        }
        let mut existing = self.list_ports().await?;
        existing.retain(|p| p.host != host);
        existing.push(PortRule { host, guest, native_id: None });
        let pairs: Vec<(u16, u16)> = existing.iter().map(|p| (p.host, p.guest)).collect();
        self.backend.edit_port_forwards(&pairs).await
            .map_err(OpsError::Other)
    }

    async fn remove_port(&self, host: u16) -> Result<(), OpsError> {
        if !self.backend.supports_port_edit() {
            return Err(OpsError::unsupported("remove_port",
                "WSL port edit unavailable on this host"));
        }
        let mut existing = self.list_ports().await?;
        let before = existing.len();
        existing.retain(|p| p.host != host);
        if existing.len() == before {
            return Err(OpsError::not_found(format!("port rule with host={host}")));
        }
        let pairs: Vec<(u16, u16)> = existing.iter().map(|p| (p.host, p.guest)).collect();
        self.backend.edit_port_forwards(&pairs).await
            .map_err(OpsError::Other)
    }

    async fn doctor(&self) -> Result<SandboxDoctorReport, OpsError> {
        let mut issues = Vec::new();
        let vm_up = matches!(self.backend.is_available().await, Ok(true));
        if !vm_up {
            issues.push(DoctorIssue {
                id: "vm-not-running".into(),
                severity: Severity::Error,
                message: "WSL2 instance is not running or wsl.exe unreachable".into(),
                repair_hint: Some("clawops sandbox start".into()),
                auto_repairable: true,
            });
        } else {
            if let Some(i) = super::probes::probe_dns(&self.backend).await { issues.push(i); }
            if let Some(i) = super::probes::probe_disk(&self.backend).await { issues.push(i); }
        }
        let host_ports: Vec<u16> = self
            .list_ports()
            .await
            .unwrap_or_default()
            .iter()
            .map(|p| p.host)
            .collect();
        if !host_ports.is_empty() {
            issues.extend(super::probes::probe_port_conflicts(&host_ports).await);
        }
        Ok(SandboxDoctorReport {
            backend: BackendKind::Wsl2,
            instance_name: self.instance_name.clone(),
            issues,
            checked_at: Utc::now().to_rfc3339(),
        })
    }

    async fn repair(&self, issue_ids: &[String], progress: ProgressSink)
        -> Result<(), OpsError>
    {
        super::repair::dispatch_repair(&self.backend, issue_ids, &progress).await
    }

    async fn stats(&self) -> Result<ResourceStats, OpsError> {
        let s = self.backend.stats().await.map_err(OpsError::Other)?;
        Ok(ResourceStats {
            cpu_percent: s.cpu_percent,
            memory_used_mb: s.memory_used_mb,
            memory_limit_mb: s.memory_limit_mb,
        })
    }

    async fn dump_logs(&self, tail: Option<u32>) -> Result<String, OpsError> {
        let n = tail.unwrap_or(DEFAULT_LOG_TAIL).to_string();
        let pipeline = format!("dmesg 2>/dev/null | tail -n {n}");
        self.backend
            .exec_argv(&["sh", "-c", &pipeline])
            .await
            .map_err(OpsError::Other)
    }
}

const DEFAULT_LOG_TAIL: u32 = 100;

// Windows-only function used by list_ports; tests run on all OSes so we
// keep it compiled but tolerate "unused" on non-Windows hosts.
#[allow(dead_code)]
/// Parse `netsh interface portproxy show v4tov4` output into PortRules.
///
/// Example output (English locale):
/// ```text
/// Listen on IPv4:             Connect to IPv4:
///
/// Address         Port        Address         Port
/// --------------- ----------  --------------- ----------
/// 0.0.0.0         3000        172.26.0.2      3000
/// 0.0.0.0         8080        172.26.0.2      8080
/// ```
///
/// netsh's output is whitespace-formatted and localization-dependent; we
/// scan for data rows by looking for lines with 4 numeric-friendly columns
/// where columns 2 and 4 are parseable as u16.
pub(crate) fn parse_netsh_portproxy(out: &str) -> Vec<PortRule> {
    let mut rules = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 4 { continue; }
        let host_port = match parts[1].parse::<u16>() { Ok(v) => v, Err(_) => continue };
        let guest_port = match parts[3].parse::<u16>() { Ok(v) => v, Err(_) => continue };
        rules.push(PortRule {
            host: host_port,
            guest: guest_port,
            native_id: Some(format!("{}:{}", parts[0], parts[1])),
        });
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox_ops::testing::MockBackend;

    #[test]
    fn backend_kind() {
        assert_eq!(WslOps::new("x").backend_kind(), BackendKind::Wsl2);
    }

    #[tokio::test]
    async fn dump_logs_uses_dmesg_tail_pipeline() {
        let mock = std::sync::Arc::new(MockBackend::new("wsl").with_stdout("ok"));
        let ops = WslOps::with_backend(mock.clone(), "test");
        let out = ops.dump_logs(Some(25)).await.unwrap();
        assert_eq!(out, "ok");
        let last = mock.last_exec().unwrap();
        assert!(last.contains("dmesg"));
        assert!(last.contains("tail -n 25"));
    }

    #[test]
    fn parse_netsh_happy_path() {
        let out = r#"
Listen on IPv4:             Connect to IPv4:

Address         Port        Address         Port
--------------- ----------  --------------- ----------
0.0.0.0         3000        172.26.0.2      3000
0.0.0.0         8080        172.26.0.2      8080
"#;
        let rules = parse_netsh_portproxy(out);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].host, 3000);
        assert_eq!(rules[0].guest, 3000);
        assert_eq!(rules[1].host, 8080);
    }

    #[test]
    fn parse_netsh_empty_returns_empty() {
        assert!(parse_netsh_portproxy("").is_empty());
        assert!(parse_netsh_portproxy("no rules here").is_empty());
    }

    #[test]
    fn parse_netsh_ignores_header_and_separator() {
        let out = r#"
Address         Port        Address         Port
--------------- ----------  --------------- ----------
"#;
        assert!(parse_netsh_portproxy(out).is_empty());
    }
}
