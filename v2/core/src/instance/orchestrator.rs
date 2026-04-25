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
use crate::common::{CancellationToken, CommandRunner, CommandSpec, OpsError, ProgressSink};
use crate::native_ops::{DefaultNativeOps, NativeOps, VersionSpec};
use crate::paths::clawenv_root;
use crate::runners::LocalProcessRunner;
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

        // Native path — dispatches to a separate pipeline. Returns early
        // with its own InstallReport.
        if opts.backend == SandboxKind::Native {
            return self.install_native(opts, provisioning, progress).await;
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

        // Dry-run short-circuit: render + describe, but do not invoke
        // limactl/wsl/podman. Returns a synthesised report with
        // install_elapsed_secs=0 and version_output="(dry-run)".
        if opts.dry_run {
            progress.at(50, "dry-run",
                "Rendered templates; stopping before backend invocation").await;
            let preview = render_dry_run_preview(&prov_create, &*provisioning, opts.backend);
            progress.at(100, "done",
                format!("dry-run complete (backend={:?})", opts.backend)).await;
            // Emit the preview in the progress stream too, so CLI users
            // see exactly what would run. One line per section.
            for line in preview.lines() {
                progress.info("preview", line).await;
            }
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
                note: "(dry-run; not persisted)".into(),
            };
            return Ok(InstallReport {
                instance: inst,
                version_output: format!("(dry-run preview)\n{preview}"),
                install_elapsed_secs: 0,
            });
        }

        backend.create(&prov_create).await
            .map_err(|e| OpsError::Other(anyhow::anyhow!("create VM failed: {e}")))?;

        // ——— Stage 4: BootVerify ———
        progress.at(60, "boot-verify",
            "Checking VM is reachable via exec").await;
        // Right-after-boot exec probe — Lima SSH ControlMaster may be
        // mid-warmup. Use the retry variant (v0.2.10 lesson).
        let probe = backend.exec_argv_with_retry(&["echo", "clawops-ok"]).await
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

        // ——— Stage 7.5: ConfigInit (one-shot post-install seed) ———
        if let Some(init_cmd) = provisioning.config_init_cmd() {
            progress.at(94, "config-init",
                format!("Seeding {} config", provisioning.cli_binary())).await;
            let full = format!("{} {}", provisioning.cli_binary(), init_cmd);
            // Best-effort: tolerate "already configured" exit codes.
            let _ = backend.exec_argv(&["sh", "-c", &full]).await;
        }

        // ——— Stage 7.6: DashboardPreBuild (Hermes-only no-op for others) ———
        crate::provisioning::dashboard::pre_build_dashboard(
            &backend, &*provisioning, &progress
        ).await?;

        // ——— Stage 7.7: MCP plugin deploy (no-op for non-MCP claws) ———
        crate::provisioning::mcp::deploy_plugins(
            &backend, &*provisioning, &progress
        ).await?;

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

    // ---- install_native (R3.1-c) ----

    /// Native-mode install: the claw runs directly on the host using
    /// v2-managed node/git. Much smaller surface than the sandbox
    /// pipeline (no VM, no mirrors, no proxy scripts) because the host
    /// already has its own networking and user.
    ///
    /// Layout: `<clawenv_root>/native/<instance_name>/` becomes the
    /// npm `--prefix`. Binary ends up at
    /// `<clawenv_root>/native/<instance_name>/bin/<claw>`.
    async fn install_native(
        &self,
        opts: InstallOpts,
        provisioning: Box<dyn crate::claw_ops::ClawProvisioning>,
        progress: ProgressSink,
    ) -> Result<InstallReport, OpsError> {
        // Currently only npm-packaged claws have a native path. Pip and
        // git_pip would need uv on PATH, which v2 doesn't bundle yet.
        if !matches!(provisioning.package_manager(), crate::claw_ops::PackageManager::Npm) {
            return Err(OpsError::unsupported(
                "install",
                format!(
                    "claw `{}` is not npm-packaged; native install only supports npm claws today",
                    opts.claw
                ),
            ));
        }

        // Preflight: node must be present. Don't auto-install here —
        // it's a surprise-y side-effect; user runs `clawcli native
        // upgrade node` explicitly first.
        progress.at(20, "preflight", "Probing host node/git").await;
        let native_ops = DefaultNativeOps::new();
        let status = native_ops.status().await?;
        let node = status.node.as_ref().ok_or_else(|| OpsError::unsupported(
            "install",
            "native install requires node on host; run `clawcli native upgrade node` first",
        ))?;

        // Resolve npm next to node. Portable Node ships npm in the
        // same `bin/` dir on unix, or as `npm.cmd` next to node.exe on Windows.
        let node_bin = node.path.clone();
        let npm_path = {
            #[cfg(target_os = "windows")]
            { node_bin.parent().unwrap().join("npm.cmd") }
            #[cfg(not(target_os = "windows"))]
            { node_bin.parent().unwrap().join("npm") }
        };
        if !npm_path.exists() {
            return Err(OpsError::unsupported(
                "install",
                format!(
                    "npm not found next to node at {}; reinstall node with `clawcli native reinstall node`",
                    npm_path.display()
                ),
            ));
        }

        // Prepare per-instance install prefix. npm will populate
        // <prefix>/lib/node_modules/<pkg> and <prefix>/bin/<claw>.
        let prefix = clawenv_root().join("native").join(&opts.name);
        tokio::fs::create_dir_all(&prefix).await
            .map_err(|e| OpsError::Other(anyhow::anyhow!(
                "create native install prefix {}: {e}", prefix.display()
            )))?;

        // Run `npm install -g --prefix <prefix> <pkg>@<version>` on the host.
        progress.at(40, "install-claw",
            format!("npm install -g --prefix {} {}@{}",
                prefix.display(), provisioning.cli_binary(), opts.claw_version)).await;
        let pkg_spec = format!("{}@{}", provisioning.cli_binary(), opts.claw_version);
        let prefix_str = prefix.to_str()
            .ok_or_else(|| OpsError::Other(anyhow::anyhow!(
                "non-UTF8 prefix {}", prefix.display()
            )))?;
        let args = [
            "install", "-g", "--prefix", prefix_str,
            "--loglevel=error", pkg_spec.as_str(),
        ];
        let runner = LocalProcessRunner::new();
        let start = std::time::Instant::now();
        let res = runner.exec(
            CommandSpec::new(
                npm_path.to_str().ok_or_else(|| OpsError::Other(
                    anyhow::anyhow!("non-UTF8 npm path")
                ))?,
                args,
            ).with_timeout(std::time::Duration::from_secs(10 * 60)),
            CancellationToken::new(),
        ).await?;
        if !res.success() {
            return Err(OpsError::Other(anyhow::anyhow!(
                "npm install failed (exit {}):\n{}",
                res.exit_code,
                res.stderr
            )));
        }
        let install_elapsed = start.elapsed();

        // Verify: run the binary's --version.
        progress.at(90, "verify-claw",
            format!("Verifying {} binary", provisioning.cli_binary())).await;
        let bin_dir = prefix.join("bin");
        let claw_bin = {
            #[cfg(target_os = "windows")]
            { bin_dir.join(format!("{}.cmd", provisioning.cli_binary())) }
            #[cfg(not(target_os = "windows"))]
            { bin_dir.join(provisioning.cli_binary()) }
        };
        if !claw_bin.exists() {
            return Err(OpsError::Other(anyhow::anyhow!(
                "post-install binary not found at {}", claw_bin.display()
            )));
        }
        let ver_res = runner.exec(
            CommandSpec::new(
                claw_bin.to_str().unwrap(),
                [provisioning.version_flag()],
            ).with_timeout(std::time::Duration::from_secs(10)),
            CancellationToken::new(),
        ).await?;
        let version_out = ver_res.stdout.trim().to_string();

        // Record.
        progress.at(97, "save-config", "Persisting instance registry").await;
        let inst = InstanceConfig {
            name: opts.name.clone(),
            claw: opts.claw,
            backend: SandboxKind::Native,
            sandbox_instance: String::new(),
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
            format!("Native install of `{}` complete", inst.name)).await;
        Ok(InstallReport {
            instance: inst,
            version_output: version_out,
            install_elapsed_secs: install_elapsed.as_secs(),
        })
    }

    // ---- upgrade (R4-a) ----

    /// Upgrade an existing instance's claw to a new version. Reuses the
    /// same background_script polling as install, but skips VM creation
    /// and mirror/proxy re-application — those are already in place.
    ///
    /// Stages (percent anchors):
    ///   5%   lookup — find registry entry
    ///   15%  ensure-up — start VM if stopped
    ///   25%  pre-verify — probe old version (reported back in report.previous_version)
    ///   30–92% install-claw — run_background_script with claw's install cmd
    ///   95%  post-verify — confirm new binary responds
    ///   100% done — stamp updated_at in registry
    ///
    /// Note: for Npm this is `npm install -g <pkg>@<new>`, which resolves
    /// to an upgrade when the package is already present. For GitPip the
    /// recipe git-clones into the fixed `/opt/<id>` path, which will
    /// collide if the dir exists. We pre-rm it here — v1's upgrade.rs
    /// has the same move.
    pub async fn upgrade(
        &self,
        opts: UpgradeOpts,
        progress: ProgressSink,
    ) -> Result<UpgradeReport, OpsError> {
        progress.at(5, "lookup", format!("Finding instance `{}`", opts.name)).await;
        let inst = self.registry.find(&opts.name).await?
            .ok_or_else(|| OpsError::not_found(format!("instance `{}`", opts.name)))?;

        let provisioning = provisioning_for(&inst.claw)
            .ok_or_else(|| OpsError::not_found(
                format!("claw `{}` (from registry)", inst.claw)
            ))?;

        // Native path — separate pipeline (no VM).
        if inst.backend == SandboxKind::Native {
            return self.upgrade_native(inst, opts, provisioning, progress).await;
        }
        let backend = Self::sandbox_backend_for(inst.backend, &inst.sandbox_instance)
            .expect("non-native backend must yield SandboxBackend");

        // Ensure VM is running — v1 makes this implicit via exec probes
        // but an idempotent start() is cheaper and more honest.
        progress.at(15, "ensure-up", "Ensuring sandbox is up").await;
        backend.start().await
            .map_err(|e| OpsError::Other(anyhow::anyhow!("start VM: {e}")))?;

        // Capture old version for the report — tolerate missing binary
        // (user may be upgrading AFTER a broken half-install).
        progress.at(25, "pre-verify", "Probing current claw version").await;
        let previous_version = backend
            .exec_argv(&["sh", "-c", &provisioning.version_check_cmd()])
            .await
            .ok()
            .map(|s| s.trim().to_string());

        // GitPip clones into /opt/<id>; if we don't pre-rm, the second
        // install errors with "destination exists". v1 hit this in
        // v0.2.x hotfix series.
        if matches!(
            provisioning.package_manager(),
            crate::claw_ops::PackageManager::GitPip { .. }
        ) {
            progress.at(28, "prep", "Removing prior git-pip install dir").await;
            let dir = format!("/opt/{}", provisioning.id());
            let rm_cmd = format!("sudo rm -rf {dir}");
            let _ = backend.exec_argv(&["sh", "-c", &rm_cmd]).await;
        }

        // Install new version — same background_script as install().
        progress.at(30, "install-claw",
            format!("Upgrading {} → {}", provisioning.display_name(), opts.to_version)).await;
        let install_cmd = provisioning.install_cmd(&opts.to_version);
        let bg_opts = BackgroundScriptOpts {
            cmd: install_cmd.as_str(),
            label: provisioning.display_name(),
            sudo: matches!(
                provisioning.package_manager(),
                crate::claw_ops::PackageManager::GitPip { .. }
                    | crate::claw_ops::PackageManager::Pip
            ),
            log_file: "/tmp/clawenv-upgrade.log",
            done_file: "/tmp/clawenv-upgrade.done",
            script_file: "/tmp/clawenv-upgrade.sh",
            pct_range: (30, 92),
            ..Default::default()
        };
        let bg_report = run_background_script(&backend, &bg_opts, &progress).await?;

        // Post-install verify.
        progress.at(95, "post-verify",
            format!("Verifying {} binary", provisioning.cli_binary())).await;
        let new_version = backend
            .exec_argv(&["sh", "-c", &provisioning.version_check_cmd()])
            .await
            .map_err(|e| OpsError::Other(anyhow::anyhow!(
                "post-upgrade version probe: {e}"
            )))?;

        // Bump updated_at in the registry.
        progress.at(98, "save-config", "Updating registry timestamp").await;
        let mut updated = inst.clone();
        updated.updated_at = Utc::now().to_rfc3339();
        self.registry.update(updated.clone()).await?;

        progress.at(100, "done",
            format!("Upgraded `{}` to {}", inst.name, new_version.trim())).await;
        Ok(UpgradeReport {
            instance: updated,
            previous_version,
            new_version: new_version.trim().to_string(),
            upgrade_elapsed_secs: bg_report.elapsed.as_secs(),
        })
    }

    // ---- launch (P1-j) ----

    /// Start an instance's gateway (and dashboard if it has one)
    /// process. Sandboxed instances spawn via `nohup ... &` inside
    /// the VM; native instances spawn detached on the host with
    /// stdout/stderr redirected to log files under `<root>/native/`.
    /// Idempotent-ish: a second launch starts another nohup process.
    /// Callers should `clawcli stop` first if they need a clean restart.
    ///
    /// Probes the gateway port (or dashboard port if dashboard exists)
    /// for readiness, up to 30s. Returns when port responds or after
    /// timeout (with a warning, not a hard failure — slow boots happen).
    pub async fn launch(&self, name: &str) -> Result<LaunchReport, OpsError> {
        let inst = self.registry.find(name).await?
            .ok_or_else(|| OpsError::not_found(format!("instance `{name}`")))?;
        let provisioning = provisioning_for(&inst.claw)
            .ok_or_else(|| OpsError::not_found(
                format!("claw `{}` (from registry)", inst.claw)
            ))?;

        let gateway_port = inst.ports.iter()
            .find(|p| p.label == "gateway")
            .map(|p| p.host)
            .unwrap_or_else(|| provisioning.default_port());

        // Determine where to spawn. Sandboxed → exec nohup inside VM.
        // Native → host-side detached spawn.
        let mut started: Vec<&'static str> = Vec::new();

        if let Some(gw_cmd) = provisioning.gateway_start_cmd(gateway_port) {
            match inst.backend {
                SandboxKind::Native => {
                    spawn_native_daemon(&inst, provisioning.cli_binary(), &gw_cmd, "gateway")
                        .await?;
                }
                _ => {
                    let backend = Self::sandbox_backend_for(inst.backend, &inst.sandbox_instance)
                        .expect("non-native backend yields SandboxBackend");
                    backend.start().await
                        .map_err(|e| OpsError::Other(anyhow::anyhow!("VM start: {e}")))?;
                    let nohup = format!(
                        "nohup {gw_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
                    );
                    backend.exec_argv(&["sh", "-c", &nohup]).await
                        .map_err(OpsError::Other)?;
                }
            }
            started.push("gateway");
        }

        // Dashboard process — Hermes only today.
        let dashboard_port = gateway_port + provisioning.dashboard_port_offset();
        if let Some(dash_cmd) = provisioning.dashboard_start_cmd(dashboard_port) {
            match inst.backend {
                SandboxKind::Native => {
                    spawn_native_daemon(&inst, provisioning.cli_binary(), &dash_cmd, "dashboard")
                        .await?;
                }
                _ => {
                    let backend = Self::sandbox_backend_for(inst.backend, &inst.sandbox_instance)
                        .expect("non-native backend yields SandboxBackend");
                    let nohup = format!(
                        "nohup {dash_cmd} > /tmp/clawenv-dashboard.log 2>&1 &"
                    );
                    backend.exec_argv(&["sh", "-c", &nohup]).await
                        .map_err(OpsError::Other)?;
                }
            }
            started.push("dashboard");
        }

        if started.is_empty() {
            // Claws like Hermes-without-dashboard expect interactive use;
            // not an error, just nothing to spawn.
            return Ok(LaunchReport {
                instance_name: inst.name,
                started_processes: started.iter().map(|s| s.to_string()).collect(),
                gateway_ready: false,
                ready_port: None,
            });
        }

        // Probe the user-facing port (dashboard if present, else gateway).
        // 30s budget, 1s tick.
        let probe_port = if provisioning.has_dashboard() {
            dashboard_port
        } else {
            gateway_port
        };
        let gateway_ready = probe_tcp_ready(probe_port, std::time::Duration::from_secs(30)).await;

        Ok(LaunchReport {
            instance_name: inst.name,
            started_processes: started.iter().map(|s| s.to_string()).collect(),
            gateway_ready,
            ready_port: if gateway_ready { Some(probe_port) } else { None },
        })
    }

    // ---- upgrade_native (P1-l) ----

    /// Native-mode upgrade: re-runs `npm install -g --prefix <inst_dir>
    /// <pkg>@<version>`. npm "install -g" naturally upgrades when the
    /// package is already there. Same path that install_native uses,
    /// but with the registry record already present.
    async fn upgrade_native(
        &self,
        inst: InstanceConfig,
        opts: UpgradeOpts,
        provisioning: Box<dyn crate::claw_ops::ClawProvisioning>,
        progress: ProgressSink,
    ) -> Result<UpgradeReport, OpsError> {
        if !matches!(provisioning.package_manager(), crate::claw_ops::PackageManager::Npm) {
            return Err(OpsError::unsupported(
                "upgrade",
                format!(
                    "claw `{}` is not npm-packaged; native upgrade only supports npm claws today",
                    inst.claw
                ),
            ));
        }

        progress.at(20, "preflight", "Probing host node/git").await;
        let native_ops = DefaultNativeOps::new();
        let status = native_ops.status().await?;
        let node = status.node.as_ref().ok_or_else(|| OpsError::unsupported(
            "upgrade",
            "native upgrade requires node on host; run `clawcli native upgrade node` first",
        ))?;
        let node_bin = node.path.clone();
        let npm_path = {
            #[cfg(target_os = "windows")]
            { node_bin.parent().unwrap().join("npm.cmd") }
            #[cfg(not(target_os = "windows"))]
            { node_bin.parent().unwrap().join("npm") }
        };

        // Probe existing version (best-effort).
        let prefix = clawenv_root().join("native").join(&inst.name);
        let bin_dir = prefix.join("bin");
        let claw_bin = {
            #[cfg(target_os = "windows")]
            { bin_dir.join(format!("{}.cmd", provisioning.cli_binary())) }
            #[cfg(not(target_os = "windows"))]
            { bin_dir.join(provisioning.cli_binary()) }
        };
        let runner = LocalProcessRunner::new();
        progress.at(30, "pre-verify", "Probing current version").await;
        let previous_version = if claw_bin.exists() {
            runner.exec(
                CommandSpec::new(claw_bin.to_str().unwrap(), [provisioning.version_flag()])
                    .with_timeout(std::time::Duration::from_secs(10)),
                CancellationToken::new(),
            ).await
            .ok()
            .filter(|r| r.success())
            .map(|r| r.stdout.trim().to_string())
        } else {
            None
        };

        progress.at(50, "install-claw",
            format!("npm install -g --prefix {} {}@{}",
                prefix.display(), provisioning.cli_binary(), opts.to_version)).await;
        let pkg_spec = format!("{}@{}", provisioning.cli_binary(), opts.to_version);
        let prefix_str = prefix.to_str()
            .ok_or_else(|| OpsError::Other(anyhow::anyhow!(
                "non-UTF8 prefix {}", prefix.display()
            )))?;
        let args = [
            "install", "-g", "--prefix", prefix_str,
            "--loglevel=error", pkg_spec.as_str(),
        ];
        let start = std::time::Instant::now();
        let res = runner.exec(
            CommandSpec::new(
                npm_path.to_str().ok_or_else(|| OpsError::Other(
                    anyhow::anyhow!("non-UTF8 npm path")
                ))?,
                args,
            ).with_timeout(std::time::Duration::from_secs(10 * 60)),
            CancellationToken::new(),
        ).await?;
        if !res.success() {
            return Err(OpsError::Other(anyhow::anyhow!(
                "npm upgrade failed (exit {}):\n{}",
                res.exit_code, res.stderr
            )));
        }
        let upgrade_elapsed = start.elapsed();

        progress.at(90, "post-verify",
            format!("Verifying {} binary", provisioning.cli_binary())).await;
        let ver_res = runner.exec(
            CommandSpec::new(claw_bin.to_str().unwrap(), [provisioning.version_flag()])
                .with_timeout(std::time::Duration::from_secs(10)),
            CancellationToken::new(),
        ).await?;
        let new_version = ver_res.stdout.trim().to_string();

        progress.at(98, "save-config", "Updating registry timestamp").await;
        let mut updated = inst.clone();
        updated.updated_at = Utc::now().to_rfc3339();
        self.registry.update(updated.clone()).await?;

        progress.at(100, "done",
            format!("Upgraded native `{}` to {}", inst.name, new_version)).await;
        Ok(UpgradeReport {
            instance: updated,
            previous_version,
            new_version,
            upgrade_elapsed_secs: upgrade_elapsed.as_secs(),
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
    /// Don't actually touch any backend. Render the templates and
    /// surface what WOULD be executed, then stop before CreateVm.
    /// Intended for CI / Gate-0 verification.
    #[serde(default)]
    pub dry_run: bool,
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
            dry_run: false,
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

// ——— Launch helpers (P1-j) ———

/// Spawn a native-mode daemon detached. stdout/stderr go to a log
/// file under `<clawenv_root>/native/<instance>/<role>.log`. Drops
/// the Child handle so the daemon outlives our process.
async fn spawn_native_daemon(
    inst: &InstanceConfig,
    bin_name: &str,
    full_cmd: &str,
    role: &'static str,
) -> Result<(), OpsError> {
    use std::process::Stdio;
    // Find the binary under <root>/native/<inst>/bin/.
    let prefix = clawenv_root().join("native").join(&inst.name);
    let bin_dir = prefix.join("bin");
    let bin_path = {
        #[cfg(target_os = "windows")]
        { bin_dir.join(format!("{bin_name}.cmd")) }
        #[cfg(not(target_os = "windows"))]
        { bin_dir.join(bin_name) }
    };
    let log_dir = clawenv_root().join("native").join(&inst.name).join("logs");
    tokio::fs::create_dir_all(&log_dir).await
        .map_err(|e| anyhow::anyhow!("create native log dir: {e}"))?;
    let log_path = log_dir.join(format!("{role}.log"));
    let log = std::fs::File::create(&log_path)
        .map_err(|e| anyhow::anyhow!("create log {}: {e}", log_path.display()))?;
    let log_clone = log.try_clone()
        .map_err(|e| anyhow::anyhow!("dup log fd: {e}"))?;

    // Parse `<bin> arg1 arg2 ...`. The `bin` token in full_cmd is the
    // claw binary name (e.g. "openclaw"); we replace with our private
    // path so PATH lookup isn't required.
    let parts: Vec<&str> = full_cmd.split_whitespace().collect();
    let args: Vec<&str> = if parts.len() > 1 { parts[1..].to_vec() } else { vec![] };

    let mut cmd = std::process::Command::new(&bin_path);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_clone));

    // Detach: drop the Child after spawn so the OS becomes its parent.
    let child = cmd.spawn()
        .map_err(|e| anyhow::anyhow!("spawn {} {}: {e}", bin_path.display(), role))?;
    drop(child);
    Ok(())
}

/// Poll `127.0.0.1:<port>` until it accepts a TCP connection or the
/// budget runs out. 1s tick. Returns true on success.
async fn probe_tcp_ready(port: u16, budget: std::time::Duration) -> bool {
    use tokio::net::TcpStream;
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        let connect = tokio::time::timeout(
            std::time::Duration::from_millis(800),
            TcpStream::connect(("127.0.0.1", port)),
        ).await;
        if let Ok(Ok(_)) = connect {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    false
}

// ——— Dry-run preview rendering ———

/// Compose a summary of what install() would execute. Called only in
/// dry_run mode — includes the rendered template head, the claw's
/// install command, and the post-install verification command.
pub(crate) fn render_dry_run_preview(
    prov: &ProvCreateOpts,
    claw: &dyn crate::claw_ops::ClawProvisioning,
    backend: SandboxKind,
) -> String {
    use crate::provisioning::{
        render_lima_yaml, render_podman_build_args, render_wsl_provision_script,
    };
    let mut out = String::new();
    out.push_str(&format!("backend         : {:?}\n", backend));
    out.push_str(&format!("instance_name   : {}\n", prov.instance_name));
    out.push_str(&format!("workspace_dir   : {}\n", prov.workspace_dir.display()));
    out.push_str(&format!("gateway_port    : {}\n", prov.gateway_port));
    out.push_str(&format!("cpu_cores       : {}\n", prov.cpu_cores));
    out.push_str(&format!("memory_mb       : {}\n", prov.memory_mb));
    out.push_str(&format!("claw_package    : {}\n", prov.claw_package));
    out.push_str(&format!("claw_version    : {}\n", prov.claw_version));
    out.push_str(&format!("install_browser : {}\n", prov.install_browser));
    match &prov.proxy {
        Some(t) => out.push_str(&format!("proxy.source    : {:?}\n", t.source)),
        None => out.push_str("proxy           : (none)\n"),
    }
    if prov.mirrors.is_default() {
        out.push_str("mirrors         : upstream default\n");
    } else {
        out.push_str(&format!(
            "mirrors         : alpine={} npm={}\n",
            prov.mirrors.alpine_repo_url(),
            prov.mirrors.npm_registry_url()
        ));
    }

    match backend {
        SandboxKind::Lima => {
            let yaml = render_lima_yaml(prov);
            out.push_str("\n---- rendered Lima YAML (first 30 lines) ----\n");
            for l in yaml.lines().take(30) {
                out.push_str(l);
                out.push('\n');
            }
            out.push_str(&format!("... ({} lines total)\n", yaml.lines().count()));
        }
        SandboxKind::Podman => {
            let args = render_podman_build_args(prov);
            out.push_str("\n---- podman build args ----\n");
            out.push_str(&format!("podman {}\n", args.join(" ")));
        }
        SandboxKind::Wsl2 => {
            let script = render_wsl_provision_script(prov);
            out.push_str("\n---- WSL provision script (first 20 lines) ----\n");
            for l in script.lines().take(20) {
                out.push_str(l);
                out.push('\n');
            }
        }
        SandboxKind::Native => {}
    }

    out.push_str("\n---- post-boot install cmd ----\n");
    out.push_str(&claw.install_cmd(&prov.claw_version));
    out.push('\n');
    out.push_str("\n---- post-install verify ----\n");
    out.push_str(&claw.version_check_cmd());
    out.push('\n');
    out
}

// ——— Upgrade opts + report ———

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeOpts {
    pub name: String,
    /// Target version or "latest".
    pub to_version: String,
}

impl UpgradeOpts {
    pub fn to_latest(name: impl Into<String>) -> Self {
        Self { name: name.into(), to_version: "latest".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeReport {
    pub instance: InstanceConfig,
    /// `<bin> --version` output from BEFORE the upgrade. `None` when
    /// the old binary couldn't be probed (broken install, missing
    /// binary, etc.) — doesn't block upgrade.
    pub previous_version: Option<String>,
    pub new_version: String,
    pub upgrade_elapsed_secs: u64,
}

// ——— Launch report (P1-j) ———

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchReport {
    pub instance_name: String,
    /// Names of processes spawned: typically `["gateway"]` or
    /// `["gateway", "dashboard"]` (or empty for interactive-only claws).
    pub started_processes: Vec<String>,
    /// True if the user-facing port (dashboard for claws that have
    /// one, else gateway) accepted a TCP connection within 30s.
    pub gateway_ready: bool,
    /// The port we probed. None when nothing was started.
    pub ready_port: Option<u16>,
}

#[cfg(test)]
// The ENV_LOCK guard is held across an await in one test — purely
// for serialization of process-global env mutation. No async code
// in the test would contend for that mutex, so the deadlock
// clippy warns about is impossible here.
#[allow(clippy::await_holding_lock)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // Tests that mutate CLAWENV_HOME must serialize — the env is
    // process-global. Mirrors the pattern in paths/mod.rs tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    async fn install_native_requires_node_on_host() {
        // Native install reaches preflight and bails because no node
        // is set up (the test env has no ~/.clawenv/node). Since the
        // test binary inherits the user's real HOME, guard against
        // false negatives by pointing CLAWENV_HOME at a fresh tmp.
        let home = TempDir::new().unwrap();
        // Serialize env mutation with other env-touching tests.
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("CLAWENV_HOME").ok();
        unsafe { std::env::set_var("CLAWENV_HOME", home.path()); }

        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.install(
            InstallOpts::minimal("test", "openclaw", SandboxKind::Native),
            ProgressSink::noop(),
        ).await.unwrap_err();

        match prev {
            Some(v) => unsafe { std::env::set_var("CLAWENV_HOME", v) },
            None => unsafe { std::env::remove_var("CLAWENV_HOME") },
        }
        match err {
            OpsError::Unsupported { what, reason } => {
                assert_eq!(what, "install");
                assert!(reason.contains("node"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    // ——— upgrade() pre-backend validation ———

    #[tokio::test]
    async fn upgrade_missing_instance_errs_not_found() {
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        let err = o.upgrade(
            UpgradeOpts::to_latest("ghost"),
            ProgressSink::noop(),
        ).await.unwrap_err();
        assert!(matches!(err, OpsError::NotFound { .. }));
    }

    #[tokio::test]
    async fn upgrade_native_requires_node_on_host() {
        // Native upgrade reaches preflight and bails because no node
        // is set up under the test CLAWENV_HOME. Mirrors install_native_*.
        let home = TempDir::new().unwrap();
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("CLAWENV_HOME").ok();
        unsafe { std::env::set_var("CLAWENV_HOME", home.path()); }

        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        o.registry.insert(InstanceConfig {
            name: "nat".into(),
            claw: "openclaw".into(),
            backend: SandboxKind::Native,
            sandbox_instance: String::new(),
            ports: vec![],
            created_at: "ts".into(),
            updated_at: String::new(),
            note: String::new(),
        }).await.unwrap();
        let err = o.upgrade(
            UpgradeOpts::to_latest("nat"),
            ProgressSink::noop(),
        ).await.unwrap_err();

        match prev {
            Some(v) => unsafe { std::env::set_var("CLAWENV_HOME", v) },
            None => unsafe { std::env::remove_var("CLAWENV_HOME") },
        }
        match err {
            OpsError::Unsupported { what, reason } => {
                assert_eq!(what, "upgrade");
                assert!(reason.contains("node"), "{reason}");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upgrade_claw_removed_from_registry_errs_not_found() {
        // Edge case: instance record has a claw id the provisioning
        // registry no longer knows (e.g. user downgraded clawcli
        // after adding a bespoke claw). Should surface cleanly.
        let tmp = TempDir::new().unwrap();
        let o = orchestrator_with_tmp_registry(&tmp);
        o.registry.insert(InstanceConfig {
            name: "orphan".into(),
            claw: "unknown-claw-xyz".into(),
            backend: SandboxKind::Lima,
            sandbox_instance: "orphan".into(),
            ports: vec![],
            created_at: "ts".into(),
            updated_at: String::new(),
            note: String::new(),
        }).await.unwrap();
        let err = o.upgrade(
            UpgradeOpts::to_latest("orphan"),
            ProgressSink::noop(),
        ).await.unwrap_err();
        assert!(matches!(err, OpsError::NotFound { .. }));
    }
}
