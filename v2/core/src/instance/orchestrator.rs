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

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::claw_ops::{provisioning_for, ClawRegistry};
use crate::common::{CancellationToken, OpsError, ProgressSink};
use crate::native_ops::{DefaultNativeOps, NativeOps, VersionSpec};
use crate::paths::clawenv_root;
use crate::provisioning::{
    apply_mirrors, run_background_script, BackgroundScriptOpts,
    CreateOpts as ProvCreateOpts, MirrorsConfig,
};
use crate::proxy::{apply::apply_to_sandbox, ProxyTriple};
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

    fn validate_backend_for_claw(
        claw: &str,
        backend: SandboxKind,
        op_name: &str,
    ) -> Result<(), OpsError> {
        let cli = ClawRegistry::cli_for(claw)
            .ok_or_else(|| OpsError::not_found(format!("claw `{claw}`")))?;
        if backend == SandboxKind::Native && !cli.supports_native() {
            return Err(OpsError::unsupported(
                op_name,
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
        Self::validate_backend_for_claw(&opts.claw, opts.backend, "create")?;

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

    // ---- install (R3-P3 8-stage pipeline) ----

    /// Full install flow: create VM → apply mirrors → apply proxy →
    /// install claw package → record instance. Each stage emits a
    /// progress event so UIs can render live state.
    ///
    /// Stage-to-percent map mirrors v1's install.rs layout:
    ///   5% validate, 10% detect_backend, 20% create_vm (up to 60%),
    ///   65% boot_verify, 70% configure_proxy (mirrors + proxy),
    ///   75–90% install_claw (background script within that range),
    ///   95% save_config, 100% done.
    pub async fn install(
        &self,
        opts: InstallOpts,
        progress: ProgressSink,
    ) -> Result<InstallReport, OpsError> {
        // ——— Stage 1: Validate ———
        progress.at(5, "validate",
            format!("Validating install(name={}, claw={})", opts.name, opts.claw)).await;
        if opts.name.is_empty() {
            return Err(OpsError::parse("instance name cannot be empty"));
        }
        if self.registry.find(&opts.name).await?.is_some() {
            return Err(OpsError::unsupported(
                "install",
                format!("instance `{}` already exists — destroy it first", opts.name),
            ));
        }
        let provisioning = provisioning_for(&opts.claw)
            .ok_or_else(|| OpsError::not_found(format!("claw `{}`", opts.claw)))?;
        Self::validate_claw(&opts.claw)?;
        Self::validate_backend_for_claw(&opts.claw, opts.backend, "install")?;
        if opts.backend == SandboxKind::Native && !provisioning.supports_native() {
            return Err(OpsError::unsupported(
                "install",
                format!("claw `{}` does not support native execution", opts.claw),
            ));
        }

        // ——— Stage 2: DetectBackend ———
        progress.at(10, "detect-backend",
            format!("Selecting backend: {:?}", opts.backend)).await;
        // Native path — R3 defers full native install orchestration.
        if opts.backend == SandboxKind::Native {
            return Err(OpsError::unsupported(
                "install",
                "native mode install not yet implemented in v2 orchestrator",
            ));
        }
        let backend = Self::sandbox_backend_for(opts.backend, &opts.name)
            .expect("non-native backend must yield SandboxBackend");

        // ——— Stage 3: CreateVm ———
        progress.at(20, "create-vm",
            format!("Creating {:?} instance `{}`", opts.backend, opts.name)).await;
        let workspace = opts.workspace_dir.clone().unwrap_or_else(|| {
            clawenv_root().join("workspaces").join(&opts.name)
        });
        let prov_create = ProvCreateOpts {
            instance_name: opts.name.clone(),
            workspace_dir: workspace.clone(),
            gateway_port: opts.gateway_port,
            cpu_cores: opts.cpu_cores,
            memory_mb: opts.memory_mb,
            proxy: opts.proxy.clone(),
            mirrors: opts.mirrors.clone(),
            claw_package: provisioning.cli_binary().to_string(),
            claw_version: opts.claw_version.clone(),
            install_browser: opts.install_browser,
        };
        backend.create(&prov_create).await
            .map_err(|e| OpsError::Other(anyhow::anyhow!("create VM failed: {e}")))?;

        // ——— Stage 4: BootVerify ———
        progress.at(60, "boot-verify",
            "Checking VM is reachable via exec").await;
        // Simple exec probe — `echo ok` must succeed.
        let probe = backend.exec_argv(&["echo", "clawops-ok"]).await
            .map_err(|e| OpsError::Other(anyhow::anyhow!("VM exec probe: {e}")))?;
        if !probe.contains("clawops-ok") {
            return Err(OpsError::Other(anyhow::anyhow!(
                "VM reachable but exec probe returned unexpected output: {probe:?}"
            )));
        }

        // ——— Stage 5: ConfigureProxy (mirrors + proxy application) ———
        progress.at(65, "configure-mirrors", "Applying apk/npm mirrors").await;
        apply_mirrors(&backend, &opts.mirrors).await?;

        if let Some(ref triple) = opts.proxy {
            progress.at(72, "configure-proxy", "Applying proxy to sandbox").await;
            apply_to_sandbox(&backend, triple).await?;
        }

        // ——— Stage 6: InstallClaw (via background_script) ———
        // Hermes needs python3-dev + uv; OpenClaw needs no extras.
        let extra_pkgs = provisioning.sandbox_provision_packages();
        if !extra_pkgs.is_empty() {
            progress.at(76, "install-deps",
                format!("Installing {} extra apk package(s)", extra_pkgs.len())).await;
            // Base packages were installed at VM create time (cloud-init /
            // WSL provision / Containerfile). Here we only add claw-specific.
            let pkgs_str = extra_pkgs.join(" ");
            let cmd = format!("sudo apk add --no-cache {pkgs_str}");
            backend.exec_argv(&["sh", "-c", &cmd]).await
                .map_err(|e| OpsError::Other(anyhow::anyhow!(
                    "extra apk install ({pkgs_str}): {e}"
                )))?;
        }

        progress.at(80, "install-claw",
            format!("Installing {} @ {}", opts.claw, opts.claw_version)).await;
        let install_cmd = provisioning.install_cmd(&opts.claw_version);
        let bg_opts = BackgroundScriptOpts {
            cmd: install_cmd.as_str(),
            label: provisioning.display_name(),
            sudo: matches!(
                provisioning.package_manager(),
                crate::claw_ops::PackageManager::GitPip { .. }
                    | crate::claw_ops::PackageManager::Pip
            ),
            pct_range: (80, 92),
            ..Default::default()
        };
        let bg_report = run_background_script(&backend, &bg_opts, &progress).await?;

        // ——— Stage 7: PostInstallVerify ———
        progress.at(93, "verify-claw",
            format!("Verifying {} binary", provisioning.cli_binary())).await;
        let ver_cmd = provisioning.version_check_cmd();
        let version_out = backend.exec_argv(&["sh", "-c", &ver_cmd]).await
            .map_err(|e| OpsError::Other(anyhow::anyhow!(
                "post-install version probe failed: {e}"
            )))?;

        // ——— Stage 8: SaveConfig ———
        progress.at(97, "save-config", "Persisting instance registry").await;
        let inst = InstanceConfig {
            name: opts.name.clone(),
            claw: opts.claw.clone(),
            backend: opts.backend,
            sandbox_instance: opts.name.clone(),
            ports: vec![PortBinding {
                host: opts.gateway_port,
                guest: opts.gateway_port,
                label: "gateway".into(),
            }],
            created_at: Utc::now().to_rfc3339(),
            updated_at: String::new(),
            note: String::new(),
        };
        self.registry.insert(inst.clone()).await?;

        progress.at(100, "done",
            format!("Instance `{}` installed ({})", inst.name, version_out.trim())).await;
        Ok(InstallReport {
            instance: inst,
            version_output: version_out.trim().to_string(),
            install_elapsed_secs: bg_report.elapsed.as_secs(),
        })
    }
}

// ——— Install opts + report ———

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOpts {
    pub name: String,
    pub claw: String,
    pub backend: SandboxKind,
    pub claw_version: String,
    pub gateway_port: u16,
    pub cpu_cores: u32,
    pub memory_mb: u32,
    #[serde(default)]
    pub install_browser: bool,
    #[serde(default)]
    pub workspace_dir: Option<PathBuf>,
    #[serde(default)]
    pub proxy: Option<ProxyTriple>,
    #[serde(default)]
    pub mirrors: MirrorsConfig,
}

impl InstallOpts {
    /// Sensible defaults for `clawcli install <claw>` without a ton of flags.
    pub fn minimal(name: impl Into<String>, claw: impl Into<String>, backend: SandboxKind) -> Self {
        Self {
            name: name.into(),
            claw: claw.into(),
            backend,
            claw_version: "latest".into(),
            gateway_port: 3000,
            cpu_cores: 2,
            memory_mb: 2048,
            install_browser: false,
            workspace_dir: None,
            proxy: None,
            mirrors: MirrorsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallReport {
    pub instance: InstanceConfig,
    /// stdout of the post-install `<binary> --version` probe.
    pub version_output: String,
    pub install_elapsed_secs: u64,
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

    // ——— install() pre-backend validation ———

    #[tokio::test]
    async fn install_empty_name_errs_parse() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.install(
            InstallOpts::minimal("", "openclaw", SandboxKind::Lima),
            ProgressSink::noop(),
        ).await.unwrap_err();
        assert!(matches!(err, OpsError::Parse(_)));
    }

    #[tokio::test]
    async fn install_unknown_claw_errs_not_found() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.install(
            InstallOpts::minimal("test", "nonexistent", SandboxKind::Lima),
            ProgressSink::noop(),
        ).await.unwrap_err();
        assert!(matches!(err, OpsError::NotFound { .. }));
    }

    #[tokio::test]
    async fn install_existing_instance_errs_unsupported() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        // Pre-seed the registry with "already" — install must refuse.
        o.registry.insert(InstanceConfig {
            name: "already".into(),
            claw: "openclaw".into(),
            backend: SandboxKind::Lima,
            sandbox_instance: "already".into(),
            ports: vec![],
            created_at: "ts".into(),
            updated_at: String::new(),
            note: String::new(),
        }).await.unwrap();

        let err = o.install(
            InstallOpts::minimal("already", "openclaw", SandboxKind::Lima),
            ProgressSink::noop(),
        ).await.unwrap_err();
        match err {
            OpsError::Unsupported { what, reason } => {
                assert_eq!(what, "install");
                assert!(reason.contains("already exists"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn install_native_for_hermes_errs_unsupported() {
        // Hermes declares supports_native=false.
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.install(
            InstallOpts::minimal("test", "hermes", SandboxKind::Native),
            ProgressSink::noop(),
        ).await.unwrap_err();
        match err {
            OpsError::Unsupported { what, reason } => {
                assert_eq!(what, "install");
                assert!(
                    reason.contains("does not support native"),
                    "{reason}"
                );
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn install_native_openclaw_deferred_to_r3_1() {
        // Orchestrator explicitly bails on native for now.
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.install(
            InstallOpts::minimal("test", "openclaw", SandboxKind::Native),
            ProgressSink::noop(),
        ).await.unwrap_err();
        match err {
            OpsError::Unsupported { reason, .. } => {
                assert!(reason.contains("native"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }
}
