//! Top-level runtime for the remote-control daemon.
//!
//! One call wires up: permission preflight, MCP server
//! (`bridge::mcp`), WSS supervisor (`remote::supervisor`), agent
//! invoker (`remote::agent`), dispatcher (`remote::dispatcher`),
//! kill-switch (`remote::killswitch`), descriptor file
//! (`bridge.mcp.json`), audit log. Returns a single handle whose
//! `stop` tears everything down in order.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::Notify;

use crate::bridge::mcp::{self, BridgeMcpDescriptor, McpServerHandle};
use crate::config::RemoteConfig;
use crate::input;
use crate::input::perm::{self, PermReport};
use crate::remote::agent::{AgentInvoker, EchoInvoker};
use crate::remote::audit::{AuditEvent, AuditLog};
use crate::remote::dispatcher::Dispatcher;
use crate::remote::killswitch::{self, KillSwitchState};
use crate::remote::supervisor::{self, SupervisorConfig, SupervisorHandle};

pub struct RuntimeHandle {
    pub mcp: McpServerHandle,
    pub supervisor: SupervisorHandle,
    pub descriptor_path: PathBuf,
    pub audit_path: PathBuf,
    pub dispatcher: tokio::task::JoinHandle<()>,
    pub kill_switch: KillSwitchState,
    pub perm_report: PermReport,
    pub shutdown: Arc<Notify>,
    /// Kept so `stop()` can log `runtime_stop` after subsystems join.
    pub audit: Arc<AuditLog>,
}

impl RuntimeHandle {
    pub async fn stop(self) {
        self.shutdown.notify_waiters();
        self.supervisor.shutdown.notify_waiters();
        let _ = self.mcp.shutdown.send(());
        let _ = self.mcp.join.await;
        let _ = self.supervisor.join.await;
        let _ = self.dispatcher.await;
        // Emit the shutdown marker AFTER all subsystems have joined so
        // the log's final line always corresponds to a real quiesced
        // state rather than "we asked them to stop".
        self.audit.log(AuditEvent::RuntimeStop);
    }
}

pub struct RuntimeOptions {
    pub remote: RemoteConfig,
    pub descriptor_path: PathBuf,
    pub audit_path: PathBuf,
    /// Agent invoker. Always required — the runtime no longer peeks at
    /// `ConfigManager::load()` to discover claw instances, to avoid
    /// the "runtime saw a different config.toml than its caller" class
    /// of bug. Callers (Tauri main, `clawcli remote daemon`) build the
    /// invoker themselves from their `AppConfig` and pass it in. See
    /// `agent::HttpGatewayInvoker::from_config` for the typical
    /// construction.
    pub invoker: Arc<dyn AgentInvoker>,
    /// When `true`, spawn the global-shortcut listener thread. Tests
    /// leave this off — `rdev::listen` is process-global and would
    /// outlive the test runtime.
    pub spawn_shortcut_listener: bool,
}

impl RuntimeOptions {
    /// Convenience for the common case: take a `RemoteConfig`, default
    /// paths, spawn the global shortcut listener, use `EchoInvoker`.
    /// Callers that want a real claw agent pass their own invoker into
    /// `RuntimeOptions` directly.
    pub fn with_echo_invoker(cfg: RemoteConfig) -> Self {
        Self {
            remote: cfg,
            descriptor_path: mcp::default_descriptor_path(),
            audit_path: AuditLog::default_path(),
            invoker: Arc::new(EchoInvoker),
            spawn_shortcut_listener: true,
        }
    }
}

pub async fn start(opts: RuntimeOptions) -> Result<RuntimeHandle> {
    let audit = Arc::new(AuditLog::open(opts.audit_path.clone()));
    audit.log(AuditEvent::RuntimeStart);

    // -------- Permission preflight --------
    let perm_report = perm::probe();
    audit.log(AuditEvent::PermProbe {
        accessibility: perm_report.accessibility.as_str().into(),
        screen_capture: perm_report.screen_capture.as_str().into(),
    });
    if let Some(guidance) = perm::guidance_for(&perm_report) {
        tracing::warn!(target: "clawenv::remote", "{}", guidance.headline);
        for step in &guidance.steps {
            tracing::warn!(target: "clawenv::remote", "  • {step}");
        }
        // Best-effort; does nothing on non-macOS.
        perm::open_guidance(&guidance);
    }

    // -------- Kill-switch --------
    let kill = KillSwitchState::new(Duration::from_secs(opts.remote.kill_switch_cooldown_sec))
        .with_audit(audit.clone());
    if opts.spawn_shortcut_listener {
        killswitch::spawn_listener(kill.clone());
    }

    // -------- MCP server --------
    let registry = input::build_default(opts.remote.input.max_screenshot_dim_px, kill.clone());
    // Snapshot tool names BEFORE moving the registry into `mcp::start` so
    // the WSS Hello's capabilities list is derived from what we actually
    // serve — no duplicated magic list that can drift out of sync when a
    // new tool is added.
    let capabilities = registry.names();
    let mcp_handle = mcp::start(
        registry,
        opts.remote.mcp.preferred_port,
        Some(audit.clone()),
    )
    .await?;

    let descriptor = BridgeMcpDescriptor {
        url: mcp_handle.url(),
        token: mcp_handle.token.clone(),
        pid: std::process::id(),
    };
    match mcp::write_descriptor(&opts.descriptor_path, &descriptor) {
        Ok(()) => {}
        // Refuse to run if another live bridge already owns the
        // descriptor — otherwise the claw agent picks up whichever
        // token landed last and the other daemon effectively goes
        // offline without warning.
        Err(e @ mcp::DescriptorError::LiveOwner { .. }) => {
            // Tear down MCP we just started so we don't leak the port.
            let _ = mcp_handle.shutdown.send(());
            return Err(anyhow::anyhow!(e));
        }
        Err(e) => {
            tracing::warn!(
                target: "clawenv::remote",
                "could not write {:?}: {e}",
                opts.descriptor_path
            );
        }
    }

    // -------- WSS supervisor --------
    let sup_cfg = SupervisorConfig {
        capabilities,
        ..SupervisorConfig::defaults(
            &opts.remote.server_url,
            &opts.remote.desktop_id,
            &opts.remote.monitor_device_id,
        )
    };
    let mut sup = supervisor::spawn(sup_cfg);

    // -------- Dispatcher --------
    let kind = opts.invoker.kind();
    tracing::info!(target: "clawenv::remote", "agent invoker = {kind}");
    audit.log(AuditEvent::AgentMode { kind });
    let inbound = sup
        .take_inbound()
        .expect("supervisor handle fresh from spawn has inbound");
    let outbound = sup.outbound.clone();
    let dispatcher = Dispatcher {
        outbound,
        audit: audit.clone(),
        invoker: opts.invoker,
    };
    let dispatcher_join = tokio::spawn(dispatcher.run(inbound));

    let shutdown = Arc::new(Notify::new());
    Ok(RuntimeHandle {
        mcp: mcp_handle,
        supervisor: sup,
        descriptor_path: opts.descriptor_path,
        audit_path: opts.audit_path,
        dispatcher: dispatcher_join,
        kill_switch: kill,
        perm_report,
        shutdown,
        audit,
    })
}
