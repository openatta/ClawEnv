//! Instance orchestration — cross-layer create / destroy workflow.
//!
//! Scope (Stage C1):
//! - `create` validates inputs, preflight-checks native runtime / sandbox
//!   availability via NativeOps / SandboxOps, registers port forwards
//!   with the sandbox, and records the instance in the registry.
//! - `destroy` removes port forwards and deletes the registry record.
//!
//! Out-of-scope (still user-driven in Stage C):
//! - Actually installing the claw binary inside the sandbox (Stage D).
//! - Creating a fresh sandbox VM from scratch (needs bootstrap flow).

use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;

use crate::claw_ops::ClawRegistry;
use crate::common::{CancellationToken, OpsError, ProgressSink};
use crate::native_ops::{DefaultNativeOps, NativeOps, VersionSpec};
use crate::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};
use crate::sandbox_ops::{LimaOps, PodmanOps, SandboxOps, WslOps};

use super::config::{InstanceConfig, InstanceRegistry, PortBinding, SandboxKind};

pub struct CreateOpts {
    pub name: String,
    pub claw: String,
    pub backend: SandboxKind,
    pub sandbox_instance: String,
    pub ports: Vec<PortBinding>,
    pub note: String,
    /// If true, auto-install missing native deps (node/git) via NativeOps.
    /// If false, fails preflight if deps are missing.
    pub autoinstall_native_deps: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateReport {
    pub instance: InstanceConfig,
    pub native_deps_installed: Vec<String>,
    pub port_forwards_configured: usize,
    /// Notes listing any manual-next-steps that v2 couldn't automate.
    pub manual_next_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DestroyReport {
    pub instance: InstanceConfig,
    pub port_forwards_cleared: bool,
}

pub struct InstanceOrchestrator {
    registry: InstanceRegistry,
}

impl Default for InstanceOrchestrator {
    fn default() -> Self { Self::new() }
}

impl InstanceOrchestrator {
    pub fn new() -> Self {
        Self { registry: InstanceRegistry::with_default_path() }
    }

    pub fn with_registry(registry: InstanceRegistry) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &InstanceRegistry { &self.registry }

    // ---- claw / backend validation ----

    fn validate_claw(claw: &str) -> Result<(), OpsError> {
        let cli = ClawRegistry::cli_for(claw)
            .ok_or_else(|| OpsError::not_found(format!("claw `{claw}`")))?;
        // Sanity: if caller picks native backend for a claw that doesn't
        // support native, reject.
        let _ = cli;
        Ok(())
    }

    fn validate_backend_for_claw(claw: &str, backend: SandboxKind) -> Result<(), OpsError> {
        let cli = ClawRegistry::cli_for(claw)
            .ok_or_else(|| OpsError::not_found(format!("claw `{claw}`")))?;
        if backend == SandboxKind::Native && !cli.supports_native() {
            return Err(OpsError::unsupported(
                "create",
                format!("claw `{claw}` does not support native (non-sandbox) execution"),
            ));
        }
        Ok(())
    }

    fn sandbox_ops_for(
        kind: SandboxKind,
        instance: &str,
    ) -> Option<Box<dyn SandboxOps>> {
        match kind {
            SandboxKind::Native => None,
            SandboxKind::Lima => Some(Box::new(LimaOps::new(instance))),
            SandboxKind::Wsl2 => Some(Box::new(WslOps::new(instance))),
            SandboxKind::Podman => Some(Box::new(PodmanOps::new(instance))),
        }
    }

    #[allow(dead_code)]
    fn sandbox_backend_for(
        kind: SandboxKind,
        instance: &str,
    ) -> Option<Arc<dyn SandboxBackend>> {
        match kind {
            SandboxKind::Native => None,
            SandboxKind::Lima => Some(Arc::new(LimaBackend::new(instance))),
            SandboxKind::Wsl2 => Some(Arc::new(WslBackend::new(instance))),
            SandboxKind::Podman => Some(Arc::new(PodmanBackend::new(instance))),
        }
    }

    // ---- create ----

    pub async fn create(
        &self,
        opts: CreateOpts,
        progress: ProgressSink,
    ) -> Result<CreateReport, OpsError> {
        progress.at(0, "validate", format!("Validating create(name={}, claw={})", opts.name, opts.claw)).await;
        if opts.name.is_empty() {
            return Err(OpsError::parse("instance name cannot be empty"));
        }
        Self::validate_claw(&opts.claw)?;
        Self::validate_backend_for_claw(&opts.claw, opts.backend)?;

        if self.registry.find(&opts.name).await?.is_some() {
            return Err(OpsError::unsupported(
                "create",
                format!("instance `{}` already exists", opts.name),
            ));
        }

        let mut installed_deps = Vec::new();
        let mut manual_next_steps = Vec::new();
        let mut ports_configured = 0;

        match opts.backend {
            SandboxKind::Native => {
                progress.at(20, "preflight", "Probing native runtime").await;
                let native_ops = DefaultNativeOps::new();
                let doctor = native_ops.doctor().await?;
                let critical: Vec<_> = doctor.issues.iter()
                    .filter(|i| matches!(
                        i.id.as_str(),
                        "node-missing" | "node-unversionable" | "git-missing" | "git-unversionable"
                    ))
                    .collect();
                if !critical.is_empty() {
                    if opts.autoinstall_native_deps {
                        progress.at(30, "install-deps", "Installing missing native deps").await;
                        for issue in &critical {
                            match issue.id.as_str() {
                                "node-missing" | "node-unversionable" => {
                                    native_ops.upgrade_node(VersionSpec::Latest, progress.clone()).await?;
                                    installed_deps.push("node".into());
                                }
                                "git-missing" | "git-unversionable" => {
                                    native_ops.upgrade_git(VersionSpec::Latest, progress.clone()).await?;
                                    installed_deps.push("git".into());
                                }
                                _ => {}
                            }
                        }
                    } else {
                        let missing: Vec<String> = critical.iter().map(|i| i.id.clone()).collect();
                        return Err(OpsError::unsupported(
                            "create",
                            format!("native preflight failed: missing {missing:?}; \
                                    rerun with --autoinstall-deps"),
                        ));
                    }
                }
                manual_next_steps.push(format!(
                    "Install `{}` on your PATH (v2 does not yet bundle claw installers)",
                    opts.claw
                ));
            }
            _ => {
                progress.at(20, "preflight", format!("Probing sandbox {:?}", opts.backend)).await;
                let sandbox_ops = Self::sandbox_ops_for(opts.backend, &opts.sandbox_instance)
                    .expect("non-native backend must yield SandboxOps");
                let status = sandbox_ops.status().await?;
                use crate::sandbox_ops::VmState;
                if status.state != VmState::Running {
                    return Err(OpsError::unsupported(
                        "create",
                        format!("sandbox instance `{}` is not running (state {:?}); \
                                 use v1 installer to bootstrap a VM, or start it first",
                                opts.sandbox_instance, status.state),
                    ));
                }
                progress.at(40, "preflight", "Running sandbox doctor").await;
                let doctor = sandbox_ops.doctor().await?;
                if !doctor.healthy() {
                    return Err(OpsError::unsupported(
                        "create",
                        format!("sandbox doctor reports errors: {} issue(s)",
                                doctor.issues.len()),
                    ));
                }
                if !opts.ports.is_empty() && sandbox_ops.capabilities().supports_port_edit {
                    progress.at(70, "ports",
                        format!("Registering {} port forwards", opts.ports.len())).await;
                    for p in &opts.ports {
                        sandbox_ops.add_port(p.host, p.guest).await?;
                    }
                    ports_configured = opts.ports.len();
                } else if !opts.ports.is_empty() {
                    manual_next_steps.push(format!(
                        "Port editing not supported by {:?}; configure ports at VM create time",
                        opts.backend
                    ));
                }
                manual_next_steps.push(format!(
                    "Install `{}` inside the sandbox (`{}`)",
                    opts.claw, opts.sandbox_instance
                ));
            }
        }

        progress.at(90, "record", "Persisting instance registry").await;
        let inst = InstanceConfig {
            name: opts.name.clone(),
            claw: opts.claw,
            backend: opts.backend,
            sandbox_instance: opts.sandbox_instance,
            ports: opts.ports,
            created_at: Utc::now().to_rfc3339(),
            updated_at: String::new(),
            note: opts.note,
        };
        self.registry.insert(inst.clone()).await?;

        progress.at(100, "done", format!("Instance `{}` created", inst.name)).await;
        Ok(CreateReport {
            instance: inst,
            native_deps_installed: installed_deps,
            port_forwards_configured: ports_configured,
            manual_next_steps,
        })
    }

    // ---- destroy ----

    pub async fn destroy(
        &self,
        name: &str,
        progress: ProgressSink,
    ) -> Result<DestroyReport, OpsError> {
        progress.at(10, "lookup", format!("Finding instance `{name}`")).await;
        let inst = self.registry.find(name).await?
            .ok_or_else(|| OpsError::not_found(format!("instance `{name}`")))?;

        let mut ports_cleared = false;
        if inst.backend != SandboxKind::Native && !inst.ports.is_empty() {
            if let Some(sandbox_ops) = Self::sandbox_ops_for(inst.backend, &inst.sandbox_instance) {
                if sandbox_ops.capabilities().supports_port_edit {
                    progress.at(50, "ports", "Removing port forwards").await;
                    for p in &inst.ports {
                        // Don't fail destroy on individual port removal errors.
                        let _ = sandbox_ops.remove_port(p.host).await;
                    }
                    ports_cleared = true;
                }
            }
        }

        progress.at(90, "remove", format!("Removing `{name}` from registry")).await;
        let removed = self.registry.remove(name).await?;

        progress.at(100, "done", "Instance destroyed").await;
        Ok(DestroyReport {
            instance: removed,
            port_forwards_cleared: ports_cleared,
        })
    }

    // ---- list / info ----

    pub async fn list(&self) -> Result<Vec<InstanceConfig>, OpsError> {
        self.registry.list().await
    }

    pub async fn info(&self, name: &str) -> Result<InstanceConfig, OpsError> {
        self.registry.find(name).await?
            .ok_or_else(|| OpsError::not_found(format!("instance `{name}`")))
    }

    pub async fn cancellable_noop(&self, _cancel: CancellationToken) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn orchestrator_with_tmp_registry(tmp: &TempDir) -> InstanceOrchestrator {
        let reg = InstanceRegistry::with_path(tmp.path().join("insts.toml"));
        InstanceOrchestrator::with_registry(reg)
    }

    #[tokio::test]
    async fn create_unknown_claw_errs_not_found() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.create(CreateOpts {
            name: "test".into(), claw: "nonexistent".into(),
            backend: SandboxKind::Lima, sandbox_instance: "x".into(),
            ports: vec![], note: String::new(),
            autoinstall_native_deps: false,
        }, ProgressSink::noop()).await.unwrap_err();
        assert!(matches!(err, OpsError::NotFound { .. }));
    }

    #[tokio::test]
    async fn create_native_for_hermes_errs_unsupported() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.create(CreateOpts {
            name: "test".into(), claw: "hermes".into(),
            backend: SandboxKind::Native, sandbox_instance: "x".into(),
            ports: vec![], note: String::new(),
            autoinstall_native_deps: false,
        }, ProgressSink::noop()).await.unwrap_err();
        match err {
            OpsError::Unsupported { what, .. } => assert_eq!(what, "create"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_empty_name_errs() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.create(CreateOpts {
            name: String::new(), claw: "hermes".into(),
            backend: SandboxKind::Lima, sandbox_instance: "x".into(),
            ports: vec![], note: String::new(),
            autoinstall_native_deps: false,
        }, ProgressSink::noop()).await.unwrap_err();
        assert!(matches!(err, OpsError::Parse(_)));
    }

    #[tokio::test]
    async fn destroy_missing_errs() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.destroy("ghost", ProgressSink::noop()).await.unwrap_err();
        assert!(matches!(err, OpsError::NotFound { .. }));
    }

    #[tokio::test]
    async fn list_empty_initially() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let list = o.list().await.unwrap();
        assert!(list.is_empty());
    }
}
