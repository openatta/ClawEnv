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

    /// Probe the VM state by parsing `limactl list <inst> --format json`.
    /// Maps Lima's status strings to v2 VmState variants:
    ///   "Running"            → Running
    ///   "Stopped"            → Stopped
    ///   "Broken" / errors    → Broken
    ///   instance dir absent  → Missing
    ///   anything else        → Unknown
    /// Returns Unknown on probe failure (limactl missing, parse error, etc.)
    /// — the caller already gracefully degrades.
    async fn probe_state(&self) -> VmState {
        use crate::common::CommandRunner;
        use crate::paths::limactl_bin;
        use crate::runners::LocalProcessRunner;
        use crate::common::CommandSpec;
        use std::time::Duration;

        // No instance dir → never created on this machine.
        if !lima_home().join(&self.instance_name).exists() {
            return VmState::Missing;
        }

        let runner = LocalProcessRunner::new();
        let spec = CommandSpec::new(
            limactl_bin(),
            ["list", &self.instance_name, "--format", "json"],
        ).with_timeout(Duration::from_secs(5));
        let res = match runner.exec(spec, CancellationToken::new()).await {
            Ok(r) => r,
            Err(_) => return VmState::Unknown,
        };
        if !res.success() {
            return VmState::Unknown;
        }
        let first = res.stdout.lines().next().unwrap_or("").trim();
        if first.is_empty() {
            return VmState::Missing;
        }
        let v: serde_json::Value = match serde_json::from_str(first) {
            Ok(v) => v,
            Err(_) => return VmState::Unknown,
        };
        match v["status"].as_str() {
            Some("Running") => VmState::Running,
            Some("Stopped") => VmState::Stopped,
            Some("Broken") => VmState::Broken,
            Some(_) | None => VmState::Unknown,
        }
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
            supports_snapshot: self.backend.supports_snapshot(),
        }
    }

    async fn status(&self) -> Result<SandboxStatus, OpsError> {
        let state = self.probe_state().await;
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
        let vm_up = matches!(self.backend.is_available().await, Ok(true));
        if !vm_up {
            issues.push(DoctorIssue {
                id: "vm-not-running".into(),
                severity: Severity::Error,
                message: "Lima VM is not running or limactl unreachable".into(),
                repair_hint: Some("clawops sandbox start".into()),
                auto_repairable: true,
            });
        } else {
            // Probes inside the VM only make sense when the VM is up.
            if let Some(i) = super::probes::probe_dns(&self.backend).await { issues.push(i); }
            if let Some(i) = super::probes::probe_disk(&self.backend).await { issues.push(i); }
        }
        // Host-side port conflict probe runs regardless of VM state — it's
        // often the reason the VM can't come up.
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
            backend: BackendKind::Lima,
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
        // Strategy: pull kernel ring buffer (dmesg) — always present on
        // Alpine, gives boot + recent kernel events. We defer to the
        // backend's quoting so the tail count is type-safe.
        let n = tail.unwrap_or(DEFAULT_LOG_TAIL).to_string();
        let combined = format!("dmesg | tail -n {n}");
        let out = self
            .backend
            .exec_argv(&["sh", "-c", &combined])
            .await
            .map_err(OpsError::Other)?;
        Ok(out)
    }

    async fn rename(&self, new_name: &str) -> Result<(), OpsError> {
        // limactl rename is in-place: VM must be stopped first. Lima
        // refuses to rename a Running VM with a clear error, but we
        // pre-stop here so the user gets one operation that "just works".
        use crate::paths::limactl_bin;
        use tokio::process::Command;

        let _ = self.backend.stop().await; // best-effort; tolerate stopped state

        let out = Command::new(limactl_bin())
            .args(["rename", &self.instance_name, new_name])
            .output().await
            .map_err(|e| OpsError::Other(anyhow::anyhow!("spawn limactl rename: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(OpsError::Other(anyhow::anyhow!(
                "limactl rename {} -> {} failed: {}",
                self.instance_name, new_name, stderr.trim()
            )));
        }
        Ok(())
    }

    async fn resize_disk(&self, new_gb: u32) -> Result<(), OpsError> {
        // Lima exposes per-VM disk resize via `limactl disk resize`,
        // but only operates on disks declared as `additionalDisks`.
        // For instances created with v2's standard template (single
        // primary disk in lima.yaml top-level), the path is to edit
        // lima.yaml's `disk:` field and restart. v2 doesn't yet edit
        // lima.yaml from the rename path — so for now, document the
        // workflow and bail with an actionable error.
        let _ = new_gb;
        Err(OpsError::Other(anyhow::anyhow!(
            "lima disk resize: edit ~/.lima/{}/lima.yaml `disk:` field then `clawcli restart {}` \
             (in-place CLI resize is a v0.5 follow-up — Lima primary disks need yaml edit + reboot)",
            self.instance_name, self.instance_name,
        )))
    }
}

const DEFAULT_LOG_TAIL: u32 = 100;

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
    use crate::sandbox_ops::testing::MockBackend;

    #[test]
    fn backend_kind() {
        let ops = LimaOps::new("test-inst");
        assert_eq!(ops.backend_kind(), BackendKind::Lima);
        assert_eq!(ops.instance_name(), "test-inst");
    }

    #[tokio::test]
    async fn dump_logs_uses_dmesg_tail_pipeline() {
        let mock = std::sync::Arc::new(MockBackend::new("lima").with_stdout("line1\nline2\n"));
        let ops = LimaOps::with_backend(mock.clone(), "test");
        let out = ops.dump_logs(Some(50)).await.unwrap();
        assert_eq!(out, "line1\nline2\n");
        let last = mock.last_exec().unwrap();
        // exec_argv default-impl quotes each fragment; we care that the
        // pipeline AND the tail count reach the shell.
        assert!(last.contains("dmesg"), "expected dmesg in cmd: {last}");
        assert!(last.contains("tail -n 50"), "expected tail count in cmd: {last}");
    }

    #[tokio::test]
    async fn dump_logs_default_tail_is_100() {
        let mock = std::sync::Arc::new(MockBackend::new("lima"));
        let ops = LimaOps::with_backend(mock.clone(), "test");
        let _ = ops.dump_logs(None).await.unwrap();
        let last = mock.last_exec().unwrap();
        assert!(last.contains("tail -n 100"), "default tail: {last}");
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
