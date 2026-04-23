//! Lima-backed SandboxOps. Wraps v2's own `sandbox_backend::LimaBackend`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use crate::common::{CancellationToken, OpsError, ProgressSink};
use crate::paths::lima_home;
use crate::sandbox_backend::{LimaBackend, SandboxBackend};

use super::ops::SandboxOps;
use super::types::{
    BackendKind, DoctorIssue, PortRule, ResourceStats, SandboxCaps, SandboxDoctorReport,
    SandboxStatus, Severity, VmState,
};

pub struct LimaOps {
    backend: Arc<dyn SandboxBackend>,
    instance_name: String,
}

impl LimaOps {
    pub fn new(instance_name: impl Into<String>) -> Self {
        let name = instance_name.into();
        Self {
            backend: Arc::new(LimaBackend::new(&name)),
            instance_name: name,
        }
    }

    /// Test-only / custom backend.
    pub fn with_backend(backend: Arc<dyn SandboxBackend>, instance_name: impl Into<String>) -> Self {
        Self { backend, instance_name: instance_name.into() }
    }

    fn lima_yaml_path(&self) -> PathBuf {
        lima_home().join(&self.instance_name).join("lima.yaml")
    }
}

#[async_trait]
impl SandboxOps for LimaOps {
    fn backend_kind(&self) -> BackendKind { BackendKind::Lima }
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
            backend: BackendKind::Lima,
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
        progress.info("sandbox", "lima start").await;
        self.backend.start().await.map_err(OpsError::Other)
    }

    async fn stop(&self, progress: ProgressSink, _cancel: CancellationToken)
        -> Result<(), OpsError>
    {
        progress.info("sandbox", "lima stop").await;
        self.backend.stop().await.map_err(OpsError::Other)
    }

    async fn restart(&self, progress: ProgressSink, cancel: CancellationToken)
        -> Result<(), OpsError>
    {
        self.stop(progress.clone(), cancel.clone()).await?;
        self.start(progress, cancel).await
    }

    async fn list_ports(&self) -> Result<Vec<PortRule>, OpsError> {
        let path = self.lima_yaml_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        Ok(parse_lima_port_forwards(&content))
    }

    async fn add_port(&self, host: u16, guest: u16) -> Result<(), OpsError> {
        if !self.backend.supports_port_edit() {
            return Err(OpsError::unsupported("add_port",
                "this Lima instance does not support port editing"));
        }
        // Merge with existing so we don't clobber prior rules.
        let mut existing = self.list_ports().await?;
        existing.retain(|p| p.host != host);  // dedupe
        existing.push(PortRule { host, guest, native_id: None });
        let pairs: Vec<(u16, u16)> = existing.iter().map(|p| (p.host, p.guest)).collect();
        self.backend.edit_port_forwards(&pairs).await
            .map_err(OpsError::Other)
    }

    async fn remove_port(&self, host: u16) -> Result<(), OpsError> {
        if !self.backend.supports_port_edit() {
            return Err(OpsError::unsupported("remove_port",
                "this Lima instance does not support port editing"));
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
        match self.backend.is_available().await {
            Ok(false) | Err(_) => issues.push(DoctorIssue {
                id: "vm-not-running".into(),
                severity: Severity::Error,
                message: "Lima VM is not running or limactl unreachable".into(),
                repair_hint: Some("clawops sandbox start".into()),
                auto_repairable: true,
            }),
            Ok(true) => {}
        }
        Ok(SandboxDoctorReport {
            backend: BackendKind::Lima,
            instance_name: self.instance_name.clone(),
            issues,
            checked_at: Utc::now().to_rfc3339(),
        })
    }

    async fn repair(&self, _issue_ids: &[String], _progress: ProgressSink)
        -> Result<(), OpsError>
    {
        Err(OpsError::unsupported("repair",
            "Lima repair actions — planned in Stage B"))
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
            "Lima log dump — planned in Stage B"))
    }
}

/// Parse a lima.yaml's `portForwards:` block. Lima's format for the entries
/// we care about is:
///
/// ```yaml
/// portForwards:
///   - guestPort: 3000
///     hostPort: 3000
///   - guestPort: 22
///     hostPort: 60022
/// ```
///
/// Lima has many other keys under `portForwards[]` (guestIP, hostIP, proto,
/// …) but for our PortRule model we only need guestPort/hostPort pairs. We
/// ignore anything we don't understand rather than hard-failing, because
/// lima.yaml is a moving target.
pub(crate) fn parse_lima_port_forwards(yaml: &str) -> Vec<PortRule> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut block_indent: Option<usize> = None;
    let mut cur_guest: Option<u16> = None;
    let mut cur_host: Option<u16> = None;
    let mut idx = 0usize;

    for raw in yaml.lines() {
        let line = raw.trim_end();
        // Empty / comment lines reset nothing.
        if line.trim().is_empty() || line.trim_start().starts_with('#') { continue; }

        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();

        if !in_block {
            if trimmed.starts_with("portForwards:") && indent == 0 {
                in_block = true;
                continue;
            }
            continue;
        }

        // Leaving the block: either dedent back to top-level, or another
        // top-level key starts.
        if indent == 0 && !trimmed.starts_with('-') {
            if let (Some(g), Some(h)) = (cur_guest, cur_host) {
                out.push(PortRule { host: h, guest: g, native_id: Some(format!("idx-{idx}")) });
            }
            cur_guest = None;
            cur_host = None;
            break;
        }

        // "- guestPort: X"   or   "- hostPort: X"
        if trimmed.starts_with("- ") {
            // flush previous entry
            if let (Some(g), Some(h)) = (cur_guest, cur_host) {
                out.push(PortRule { host: h, guest: g, native_id: Some(format!("idx-{idx}")) });
                idx += 1;
            }
            cur_guest = None;
            cur_host = None;
            block_indent = Some(indent);
            let after_dash = trimmed.trim_start_matches("- ").trim();
            apply_kv(after_dash, &mut cur_guest, &mut cur_host);
            continue;
        }

        // Continuation of an entry (same indent as "- " but without the dash).
        if let Some(bi) = block_indent {
            if indent >= bi + 2 {
                apply_kv(trimmed, &mut cur_guest, &mut cur_host);
            }
        }
    }

    // Flush last entry (if block extended to EOF).
    if let (Some(g), Some(h)) = (cur_guest, cur_host) {
        out.push(PortRule { host: h, guest: g, native_id: Some(format!("idx-{idx}")) });
    }

    out
}

fn apply_kv(line: &str, guest: &mut Option<u16>, host: &mut Option<u16>) {
    if let Some(rest) = line.strip_prefix("guestPort:") {
        if let Ok(v) = rest.trim().parse::<u16>() { *guest = Some(v); }
    } else if let Some(rest) = line.strip_prefix("hostPort:") {
        if let Ok(v) = rest.trim().parse::<u16>() { *host = Some(v); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind() {
        let ops = LimaOps::new("test-inst");
        assert_eq!(ops.backend_kind(), BackendKind::Lima);
        assert_eq!(ops.instance_name(), "test-inst");
    }

    #[test]
    fn parse_empty_yaml_returns_none() {
        assert!(parse_lima_port_forwards("").is_empty());
        assert!(parse_lima_port_forwards("cpus: 4\nmemory: 8GiB\n").is_empty());
    }

    #[test]
    fn parse_simple_block() {
        let y = r#"
cpus: 4
portForwards:
  - guestPort: 3000
    hostPort: 3000
  - guestPort: 22
    hostPort: 60022
memory: 8GiB
"#;
        let ports = parse_lima_port_forwards(y);
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].host, 3000);
        assert_eq!(ports[0].guest, 3000);
        assert_eq!(ports[1].host, 60022);
        assert_eq!(ports[1].guest, 22);
    }

    #[test]
    fn parse_block_with_extra_keys_ignored() {
        let y = r#"
portForwards:
  - guestPort: 8080
    hostPort: 8080
    proto: tcp
    guestIP: "0.0.0.0"
"#;
        let ports = parse_lima_port_forwards(y);
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].host, 8080);
    }

    #[test]
    fn parse_block_extends_to_eof() {
        let y = "portForwards:\n  - guestPort: 5000\n    hostPort: 5000\n";
        let ports = parse_lima_port_forwards(y);
        assert_eq!(ports.len(), 1);
    }

    #[test]
    fn parse_incomplete_entry_skipped() {
        // Missing hostPort → should be skipped, not produce a partial rule.
        let y = r#"
portForwards:
  - guestPort: 9000
  - guestPort: 7000
    hostPort: 7000
"#;
        let ports = parse_lima_port_forwards(y);
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].guest, 7000);
    }
}
