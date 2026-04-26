//! Verb-layer commands — the user-facing task surface.
//!
//! Each verb here is a thin composer over one or more Ops modules. If
//! you find yourself reaching past the Ops layer into raw shell or
//! backend APIs, that's a smell — probably missing an Ops method.
//!
//! Instance resolution rule (used by every verb that takes `[<name>]`):
//!
//! 1. Positional arg → highest precedence
//! 2. `--instance` global flag → next
//! 3. Falls back to "default" (which is what `ctx.instance` already resolves to)

use std::sync::Arc;

use clawops_core::download_ops::{CatalogBackedDownloadOps, DownloadOps};
use clawops_core::instance::{
    InstallOpts, InstanceConfig, InstanceOrchestrator, InstanceRegistry, SandboxKind,
    UpgradeOpts,
};
use clawops_core::native_ops::{DefaultNativeOps, NativeOps};
use clawops_core::preflight;
use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};
use clawops_core::sandbox_ops::{
    LimaOps, PodmanOps, SandboxDoctorReport, SandboxOps, WslOps,
};
use clawops_core::{CancellationToken, ProgressEvent, ProgressSink};
use serde::Serialize;

use crate::shared::{new_table, severity_color, Ctx};

/// Target an instance — positional wins, else ctx.instance fallback.
fn resolve_name(name: Option<String>, ctx: &Ctx) -> String {
    name.unwrap_or_else(|| ctx.instance.clone())
}

/// Look up an instance record, or synthesise a minimal one so commands
/// work even when the registry is empty (v2 can manage VMs created by
/// v1 without requiring re-registration).
async fn resolve_instance(name: &str) -> anyhow::Result<InstanceConfig> {
    let reg = InstanceRegistry::with_default_path();
    if let Some(cfg) = reg.find(name).await? {
        return Ok(cfg);
    }
    // No record: assume this is a sandbox instance the user created via
    // v1, with backend = host default. Callers who need strict behaviour
    // can pre-check with `clawcli instance info <name>`.
    let backend = default_backend_for_host();
    Ok(InstanceConfig {
        name: name.into(),
        claw: String::new(),
        backend,
        sandbox_instance: name.into(),
        ports: Vec::new(),
        created_at: String::new(),
        updated_at: String::new(),
        note: "(unregistered — synthesized from host defaults)".into(),
    })
}

fn default_backend_for_host() -> SandboxKind {
    if cfg!(target_os = "macos") { SandboxKind::Lima }
    else if cfg!(target_os = "windows") { SandboxKind::Wsl2 }
    else { SandboxKind::Podman }
}

/// Resolve the ProxyTriple for an install, combining v1 config.toml
/// settings with the current process env and backend-specific
/// loopback rewriting (Lima→host.lima.internal, Podman→
/// host.containers.internal, etc.).
///
/// Returns `None` when neither config nor env has a proxy configured.
async fn resolve_install_proxy(
    cfg: &clawops_core::proxy::ProxyConfig,
    backend: SandboxKind,
) -> Option<clawops_core::proxy::ProxyTriple> {
    use clawops_core::proxy::Scope;
    use clawops_core::sandbox_ops::BackendKind;
    let backend_kind = match backend {
        SandboxKind::Lima => BackendKind::Lima,
        SandboxKind::Wsl2 => BackendKind::Wsl2,
        SandboxKind::Podman => BackendKind::Podman,
        SandboxKind::Native => return None, // native doesn't need sandbox rewrites
    };
    Scope::RuntimeSandbox { backend: backend_kind, instance: None }
        .resolve(cfg, None)
        .await
}

fn sandbox_ops_for(cfg: &InstanceConfig) -> Option<Box<dyn SandboxOps>> {
    let target = if cfg.sandbox_instance.is_empty() {
        cfg.name.as_str()
    } else {
        cfg.sandbox_instance.as_str()
    };
    match cfg.backend {
        SandboxKind::Native => None,
        SandboxKind::Lima => Some(Box::new(LimaOps::new(target))),
        SandboxKind::Wsl2 => Some(Box::new(WslOps::new(target))),
        SandboxKind::Podman => Some(Box::new(PodmanOps::new(target))),
    }
}

fn backend_arc(cfg: &InstanceConfig) -> Option<Arc<dyn SandboxBackend>> {
    let target = if cfg.sandbox_instance.is_empty() {
        cfg.name.as_str()
    } else {
        cfg.sandbox_instance.as_str()
    };
    match cfg.backend {
        SandboxKind::Native => None,
        SandboxKind::Lima => Some(Arc::new(LimaBackend::new(target))),
        SandboxKind::Wsl2 => Some(Arc::new(WslBackend::new(target))),
        SandboxKind::Podman => Some(Arc::new(PodmanBackend::new(target))),
    }
}

// ——— status ———

/// `clawcli status [<name>] [--no-probe]` — emit `wire::StatusResponse`.
/// The summary is flattened inline (`s.name` not `s.summary.name`).
/// `--no-probe` skips the VM-state and capability probes — registry-only,
/// useful in scripts that just want to confirm a record exists.
pub async fn run_status(
    ctx: &Ctx,
    name: Option<String>,
    no_probe: bool,
) -> anyhow::Result<()> {
    let n = resolve_name(name, ctx);
    let reg = InstanceRegistry::with_default_path();
    let cfg = match reg.find(&n).await? {
        Some(c) => c,
        None => resolve_instance(&n).await?,
    };

    let (vm_state, caps) = if no_probe {
        (None, None)
    } else {
        match sandbox_ops_for(&cfg) {
            Some(ops) => (
                ops.status().await.ok().map(|s| match s.state {
                    clawops_core::sandbox_ops::VmState::Running => "running",
                    clawops_core::sandbox_ops::VmState::Stopped => "stopped",
                    clawops_core::sandbox_ops::VmState::Broken => "broken",
                    clawops_core::sandbox_ops::VmState::Missing => "missing",
                    clawops_core::sandbox_ops::VmState::Unknown => "unknown",
                }),
                Some(ops.capabilities()),
            ),
            None => (None, None),
        }
    };

    let summary = clawops_core::wire::InstanceSummary::from_instance(&cfg, vm_state);
    let resp = clawops_core::wire::StatusResponse {
        summary,
        capabilities: caps.map(|c| clawops_core::wire::CapabilitiesInfo {
            rename: c.supports_rename,
            resource_edit: c.supports_resource_edit,
            port_edit: c.supports_port_edit,
            snapshot: c.supports_snapshot,
        }),
    };
    ctx.emit_pretty(&resp, |r| {
        println!("Instance  : {}", r.summary.name);
        println!("Claw      : {}", if r.summary.claw.is_empty() { "—" } else { r.summary.claw.as_str() });
        println!("Backend   : {}", r.summary.backend);
        println!("Health    : {}", r.summary.health);
        if r.summary.gateway_port != 0 {
            println!("Gateway   : http://127.0.0.1:{}", r.summary.gateway_port);
        }
        if r.summary.dashboard_port != 0 {
            println!("Dashboard : http://127.0.0.1:{}", r.summary.dashboard_port);
        }
        if let Some(c) = &r.capabilities {
            println!("Caps      : rename={} resource_edit={} port_edit={} snapshot={}",
                c.rename, c.resource_edit, c.port_edit, c.snapshot);
        }
    })?;
    Ok(())
}

/// `clawcli info <name>` — registry-only inspection (never probes
/// the backend). Emits the raw `InstanceConfig` so scripts can read
/// the persisted record without race conditions on a live VM.
pub async fn run_info(ctx: &Ctx, name: String) -> anyhow::Result<()> {
    let reg = InstanceRegistry::with_default_path();
    let cfg = reg.find(&name).await?
        .ok_or_else(|| anyhow::anyhow!(
            "instance `{name}` not in registry (try `clawcli list`)"
        ))?;
    ctx.emit_pretty(&cfg, |c| {
        println!("Name              : {}", c.name);
        println!("Claw              : {}", if c.claw.is_empty() { "—" } else { c.claw.as_str() });
        println!("Backend           : {}", c.backend.as_str());
        println!("Sandbox instance  : {}",
            if c.sandbox_instance.is_empty() { "—" } else { c.sandbox_instance.as_str() });
        if !c.ports.is_empty() {
            println!("Ports             :");
            for p in &c.ports {
                println!("  {} → host:{} guest:{}", p.label, p.host, p.guest);
            }
        }
        println!("Created at        : {}", c.created_at);
        if !c.updated_at.is_empty() && c.updated_at != c.created_at {
            println!("Updated at        : {}", c.updated_at);
        }
        if !c.note.is_empty() { println!("Note              : {}", c.note); }
    })?;
    Ok(())
}

// ——— lifecycle: start / stop / restart ———

async fn lifecycle_dispatch(
    ctx: &Ctx,
    name: Option<String>,
    verb: &'static str,
) -> anyhow::Result<()> {
    let n = resolve_name(name, ctx);
    let cfg = resolve_instance(&n).await?;
    let ops = sandbox_ops_for(&cfg).ok_or_else(|| {
        anyhow::anyhow!("instance `{n}` is native (no VM to {verb})")
    })?;
    let cancel = CancellationToken::new();
    match verb {
        "start"   => ops.start(ProgressSink::noop(), cancel).await?,
        "stop"    => ops.stop(ProgressSink::noop(), cancel).await?,
        "restart" => ops.restart(ProgressSink::noop(), cancel).await?,
        _ => unreachable!(),
    }
    // Past tense: "stop" → "stopped" (double-p), "start" → "started",
    // "restart" → "restarted". Inline match keeps it explicit and
    // avoids the format!("{verb}ed") that produced "stoped".
    let past = match verb {
        "start" => "started",
        "stop" => "stopped",
        "restart" => "restarted",
        _ => verb,
    };
    ctx.emit_text(format!("{past} {n}"));
    Ok(())
}

pub async fn run_start(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    lifecycle_dispatch(ctx, name, "start").await
}
pub async fn run_stop(
    ctx: &Ctx,
    name: Option<String>,
    all: bool,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    if all {
        return run_stop_all(ctx).await;
    }
    // Wrap the per-backend stop call in a wall-clock timeout. Each
    // backend's internal stop logic (limactl stop, wsl --terminate,
    // podman stop) bakes its own per-step timeout, but the CLI's
    // `--timeout-secs` is the user-facing budget — if a Lima VM hangs
    // on `kill -TERM`, the user shouldn't have to ctrl-C after 5 min.
    let dur = std::time::Duration::from_secs(timeout_secs);
    let dispatch = lifecycle_dispatch(ctx, name, "stop");
    match tokio::time::timeout(dur, dispatch).await {
        Ok(result) => result,
        Err(_) => anyhow::bail!(
            "stop timed out after {}s — backend may need manual cleanup. \
             Try `clawcli sandbox stop` or the backend's CLI directly.",
            timeout_secs
        ),
    }
}
pub async fn run_restart(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    lifecycle_dispatch(ctx, name, "restart").await
}

/// `clawcli stop --all` — best-effort stop of every registered instance.
/// Per-instance failures get logged but don't abort the loop, so a
/// stuck VM can't block other VMs from shutting down.
async fn run_stop_all(ctx: &Ctx) -> anyhow::Result<()> {
    let reg = InstanceRegistry::with_default_path();
    let instances = reg.list().await?;
    let mut report: Vec<serde_json::Value> = Vec::with_capacity(instances.len());
    for inst in instances {
        if matches!(inst.backend, SandboxKind::Native) {
            report.push(serde_json::json!({
                "instance": inst.name,
                "status": "skipped",
                "reason": "native (no VM to stop)",
            }));
            continue;
        }
        let target = if inst.sandbox_instance.is_empty() {
            inst.name.clone()
        } else {
            inst.sandbox_instance.clone()
        };
        let ops: Box<dyn SandboxOps> = match inst.backend {
            SandboxKind::Lima => Box::new(LimaOps::new(target.as_str())),
            SandboxKind::Wsl2 => Box::new(WslOps::new(target.as_str())),
            SandboxKind::Podman => Box::new(PodmanOps::new(target.as_str())),
            SandboxKind::Native => unreachable!("native skipped above"),
        };
        match ops.stop(ProgressSink::noop(), CancellationToken::new()).await {
            Ok(()) => report.push(serde_json::json!({
                "instance": inst.name, "status": "stopped",
            })),
            Err(e) => report.push(serde_json::json!({
                "instance": inst.name,
                "status": "error",
                "reason": format!("{e}"),
            })),
        }
    }
    ctx.emit_pretty(&report, |rs| {
        for r in rs {
            println!("{} : {}",
                r["instance"].as_str().unwrap_or("?"),
                r["status"].as_str().unwrap_or("?"));
            if let Some(reason) = r.get("reason").and_then(|v| v.as_str()) {
                println!("  reason: {reason}");
            }
        }
    })?;
    Ok(())
}

// ——— token (read gateway token from inside VM) ———

pub async fn run_token(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    let n = resolve_name(name, ctx);
    let cfg = resolve_instance(&n).await?;
    let backend = backend_arc(&cfg).ok_or_else(|| {
        anyhow::anyhow!("instance `{n}` is native (no sandbox VM to read token from)")
    })?;
    let token = clawops_core::bridge::read_gateway_token(&backend, &cfg.claw).await?;
    if ctx.json {
        ctx.output.emit(crate::output::CliEvent::Data {
            data: serde_json::json!({"instance": n, "token": token}),
        });
    } else {
        println!("{token}");
    }
    Ok(())
}

// ——— uninstall (end-to-end teardown) ———

#[derive(Debug, clap::Args)]
pub struct UninstallArgs {
    /// Instance name to remove. Required (no fallback to ctx.instance —
    /// destroying "default" by accident is the common footgun this guards).
    pub name: String,
    /// Capture a portable bundle before destroying the VM. Matches
    /// `clawcli export` semantics; on failure we abort uninstall so
    /// the user can retry without losing the VM.
    #[arg(long)]
    pub keep_bundle: Option<std::path::PathBuf>,
}

pub async fn run_uninstall(ctx: &Ctx, args: UninstallArgs) -> anyhow::Result<()> {
    if let Some(out) = &args.keep_bundle {
        // Snapshot the VM before destroying it. Re-uses the same export
        // pipeline as `clawcli export` so output bundles are identical.
        let export_args = ExportArgs {
            name: Some(args.name.clone()),
            output: out.clone(),
        };
        ctx.emit_text(format!(
            "uninstall: capturing bundle → {} before destroy",
            out.display(),
        ));
        run_export(ctx, export_args).await?;
    }

    // Stream progress just like `instance destroy` does so the GUI's
    // delete-progress dialog gets the same events.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressEvent>(32);
    let sink = ProgressSink::new(tx);
    let printer = {
        let output = ctx.output.clone();
        let no_progress = ctx.no_progress;
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if !no_progress { emit_progress(&output, &ev); }
            }
        })
    };
    let o = InstanceOrchestrator::new();
    let report = o.destroy(&args.name, sink).await?;
    let _ = printer.await;
    ctx.emit(&report)?;
    Ok(())
}

// ——— logs ———

/// `clawcli logs <name> [--follow] [--tail N] [--since <dur>]`
/// Concatenates gateway + dashboard log files inside the sandbox VM
/// (or the host-side native install dir). `--follow` keeps the call
/// open and emits each new line as it lands; ctrl-C / SIGINT exits.
/// `--since` is reserved (advisory only — stripped from input today).
pub async fn run_logs(
    ctx: &Ctx,
    name: String,
    follow: bool,
    tail: u32,
    _since: Option<String>,
) -> anyhow::Result<()> {
    let cfg = resolve_instance(&name).await?;

    if follow {
        return run_logs_follow(ctx, &cfg, tail).await;
    }

    if matches!(cfg.backend, SandboxKind::Native) {
        // Native: log files live under ~/.clawenv/native/<name>/logs/
        let log_dir = clawops_core::paths::clawenv_root()
            .join("native").join(&cfg.name).join("logs");
        let mut combined = String::new();
        for f in ["gateway.log", "dashboard.log"] {
            let p = log_dir.join(f);
            if p.exists() {
                if let Ok(s) = tokio::fs::read_to_string(&p).await {
                    let lines: Vec<&str> = s.lines().rev().take(tail as usize).collect();
                    combined.push_str(&format!("=== {f} ===\n"));
                    for line in lines.iter().rev() {
                        combined.push_str(line);
                        combined.push('\n');
                    }
                }
            }
        }
        emit_log_response(ctx, combined);
        return Ok(());
    }

    let backend = backend_arc(&cfg).ok_or_else(|| anyhow::anyhow!(
        "instance `{name}` has no backend to read logs from"
    ))?;
    // Concatenate the standard log paths inside the VM. Failures are
    // tolerated per-file so a missing dashboard log doesn't hide the
    // gateway log.
    let cmd = format!(
        "for f in /tmp/clawenv-gateway.log /tmp/clawenv-dashboard.log /tmp/openclaw/openclaw-*.log; do \
           if [ -f \"$f\" ]; then echo \"=== $f ===\"; tail -n {tail} \"$f\" 2>/dev/null; fi; \
         done"
    );
    let out = backend.exec_argv(&["sh", "-c", &cmd]).await
        .unwrap_or_else(|e| format!("(log read failed: {e})"));
    emit_log_response(ctx, out);
    Ok(())
}

/// Streaming follow path. For sandboxed instances we exec
/// `tail -n <tail> -F <files>` inside the VM via ExecutionContext's
/// streaming surface; for native we tail the host-side files. Each
/// emitted line goes out as its own Data event (one LogResponse per
/// line) so the GUI's tail panel can scroll without buffering.
async fn run_logs_follow(
    ctx: &Ctx,
    cfg: &InstanceConfig,
    tail: u32,
) -> anyhow::Result<()> {
    use clawops_core::exec_context::for_instance;

    if matches!(cfg.backend, SandboxKind::Native) {
        // Tail host-side files via the host shell. tail -F handles
        // log rotation / file recreation, which `gateway.log` triggers
        // when the daemon restarts.
        let log_dir = clawops_core::paths::clawenv_root()
            .join("native").join(&cfg.name).join("logs");
        let g = log_dir.join("gateway.log");
        let d = log_dir.join("dashboard.log");
        let mut argv: Vec<String> = vec!["tail".into(), "-n".into(), tail.to_string(), "-F".into()];
        for p in [&g, &d] {
            if p.exists() { argv.push(p.to_string_lossy().into_owned()); }
        }
        if argv.len() == 4 {
            anyhow::bail!("no log files yet under {} (start the daemons first)", log_dir.display());
        }
        return stream_tail(ctx, argv).await;
    }

    use clawops_core::sandbox_ops::BackendKind;
    let backend_kind = match cfg.backend {
        SandboxKind::Lima => BackendKind::Lima,
        SandboxKind::Wsl2 => BackendKind::Wsl2,
        SandboxKind::Podman => BackendKind::Podman,
        SandboxKind::Native => unreachable!("native handled above"),
    };
    let target = if cfg.sandbox_instance.is_empty() { &cfg.name } else { &cfg.sandbox_instance };
    let exec_ctx = for_instance(backend_kind, target, None)
        .ok_or_else(|| anyhow::anyhow!(
            "instance `{}` has no execution context to follow logs in", cfg.name
        ))?;
    let cmd = format!(
        "tail -n {tail} -F /tmp/clawenv-gateway.log /tmp/clawenv-dashboard.log 2>/dev/null"
    );
    let mut on_line = |line: String| {
        emit_log_line(ctx, &line);
    };
    exec_ctx.exec_streaming(&["sh", "-c", &cmd], &mut on_line).await
        .map_err(|e| anyhow::anyhow!("follow: {e:?}"))?;
    Ok(())
}

/// Run `tail` as a host process, forward stdout line-by-line.
async fn stream_tail(ctx: &Ctx, argv: Vec<String>) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take()
        .ok_or_else(|| anyhow::anyhow!("tail child had no stdout pipe"))?;
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        emit_log_line(ctx, &line);
    }
    let _ = child.wait().await;
    Ok(())
}

fn emit_log_line(ctx: &Ctx, line: &str) {
    if ctx.json {
        let resp = clawops_core::wire::LogResponse { content: line.to_string() };
        ctx.output.emit(crate::output::CliEvent::Data {
            data: serde_json::to_value(&resp).expect("LogResponse serialises"),
        });
    } else {
        println!("{line}");
    }
}

fn emit_log_response(ctx: &Ctx, content: String) {
    if ctx.json {
        let resp = clawops_core::wire::LogResponse { content };
        ctx.output.emit(crate::output::CliEvent::Data {
            data: serde_json::to_value(&resp).expect("LogResponse serialises"),
        });
    } else {
        print!("{content}");
    }
}

// ——— list ———

/// `clawcli list [--filter <BACKEND>] [--include-broken]` — emit
/// `wire::ListResponse`. `--filter` matches the backend kind string
/// (`lima`/`wsl2`/`podman`/`native`). `--include-broken` keeps
/// instances whose VM probe surfaces `broken`/`missing`; by default
/// they're hidden so the list is what's usable.
pub async fn run_list(
    ctx: &Ctx,
    filter: Option<String>,
    include_broken: bool,
) -> anyhow::Result<()> {
    let reg = InstanceRegistry::with_default_path();
    let mut instances = reg.list().await?;
    if let Some(f) = filter {
        let want = f.to_ascii_lowercase();
        instances.retain(|i| i.backend.as_str() == want);
    }
    let mut summaries: Vec<clawops_core::wire::InstanceSummary> = Vec::with_capacity(instances.len());
    for inst in &instances {
        let vm_state: Option<&'static str> = match sandbox_ops_for(inst) {
            Some(ops) => match ops.status().await {
                Ok(s) => Some(match s.state {
                    clawops_core::sandbox_ops::VmState::Running => "running",
                    clawops_core::sandbox_ops::VmState::Stopped => "stopped",
                    clawops_core::sandbox_ops::VmState::Broken => "broken",
                    clawops_core::sandbox_ops::VmState::Missing => "missing",
                    clawops_core::sandbox_ops::VmState::Unknown => "unknown",
                }),
                Err(_) => Some("unknown"),
            },
            None => None,
        };
        summaries.push(clawops_core::wire::InstanceSummary::from_instance(inst, vm_state));
    }
    if !include_broken {
        summaries.retain(|s| s.health != "broken" && s.health != "missing");
    }
    let resp = clawops_core::wire::ListResponse { instances: summaries };
    ctx.emit_pretty(&resp, |r| {
        if r.instances.is_empty() {
            println!("No instances registered. Use `clawcli install <claw>` to create one.");
            return;
        }
        let mut t = new_table(["name", "claw", "backend", "health", "gateway", "sandbox"]);
        for i in &r.instances {
            t.add_row([
                i.name.clone(),
                if i.claw.is_empty() { "—".into() } else { i.claw.clone() },
                i.backend.clone(),
                i.health.clone(),
                if i.gateway_port == 0 { "—".into() } else { i.gateway_port.to_string() },
                if i.sandbox_instance.is_empty() { "—".into() } else { i.sandbox_instance.clone() },
            ]);
        }
        println!("{t}");
    })?;
    Ok(())
}

// ——— exec ———

/// `clawcli exec [<name>] -- <cmd> [args...]` — run a non-interactive
/// command inside an instance. Routes through `ExecutionContext` so
/// both sandboxed VMs and native instances are reachable through the
/// same code path. For an attached TTY use `clawcli shell`.
pub async fn run_exec(
    ctx: &Ctx,
    name: Option<String>,
    cmd: Vec<String>,
) -> anyhow::Result<()> {
    use clawops_core::exec_context::{for_instance, for_native};
    use clawops_core::sandbox_ops::BackendKind;
    if cmd.is_empty() {
        anyhow::bail!("exec: no command given (use `clawcli exec <name> -- cmd args...`)");
    }
    let n = resolve_name(name, ctx);
    let cfg = resolve_instance(&n).await?;

    let exec_ctx = match cfg.backend {
        SandboxKind::Native => {
            let prefix = clawops_core::paths::clawenv_root().join("native").join(&cfg.name);
            for_native(prefix)
        }
        SandboxKind::Lima | SandboxKind::Wsl2 | SandboxKind::Podman => {
            let backend_kind = match cfg.backend {
                SandboxKind::Lima => BackendKind::Lima,
                SandboxKind::Wsl2 => BackendKind::Wsl2,
                SandboxKind::Podman => BackendKind::Podman,
                SandboxKind::Native => unreachable!(),
            };
            let target = if cfg.sandbox_instance.is_empty() { &cfg.name } else { &cfg.sandbox_instance };
            for_instance(backend_kind, target, None).ok_or_else(|| {
                anyhow::anyhow!("instance `{n}` has no execution context (backend impl missing)")
            })?
        }
    };

    let argv_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    let stdout = exec_ctx.exec(&argv_refs).await
        .map_err(|e| anyhow::anyhow!("exec in {n}: {e:?}"))?;
    if ctx.json {
        let resp = clawops_core::wire::ExecResult {
            stdout, stderr: String::new(), exit_code: 0,
        };
        ctx.emit(&resp)?;
    } else {
        // Raw stdout, unformatted — users piping to other tools expect this.
        print!("{stdout}");
    }
    Ok(())
}

// ——— shell (interactive) ———

pub async fn run_shell(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    let n = resolve_name(name, ctx);
    let cfg = resolve_instance(&n).await?;

    // Interactive shell needs a real TTY — we can't go through CommandRunner.
    // Dispatch to the backend-native shell command directly.
    let target = if cfg.sandbox_instance.is_empty() { &cfg.name } else { &cfg.sandbox_instance };
    let (prog, args): (&str, Vec<String>) = match cfg.backend {
        SandboxKind::Native => {
            anyhow::bail!("instance `{n}` is native (no sandbox shell to enter)");
        }
        SandboxKind::Lima => ("limactl", vec!["shell".into(), target.clone()]),
        SandboxKind::Wsl2 => ("wsl", vec!["-d".into(), target.clone()]),
        SandboxKind::Podman => (
            "podman",
            vec!["exec".into(), "-it".into(), target.clone(), "/bin/sh".into()],
        ),
    };

    // Move to a blocking task so the caller's async runtime can
    // continue; inherit stdin/stdout/stderr for true interactivity.
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new(prog).args(&args).status()
    })
    .await
    .map_err(|e| anyhow::anyhow!("shell spawn join: {e}"))?
    .map_err(|e| anyhow::anyhow!("shell spawn: {e}"))?;
    if !status.success() {
        anyhow::bail!(
            "shell exited with code {}",
            status.code().unwrap_or(-1)
        );
    }
    Ok(())
}

// ——— install (end-to-end provisioning pipeline) ———

/// Backend selector for `clawcli install`. All four backends are
/// reachable here; the orchestrator dispatches `Native` to a dedicated
/// `install_native` pipeline (no VM creation; deploy onto host node/git
/// under `~/.clawenv/`). Claws that don't support native execution
/// (e.g. Hermes — see `ClawProvisioning::supports_native`) bail at
/// validation time with a clear error.
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum InstallBackendSel {
    Native,
    Lima,
    Wsl2,
    Podman,
}

impl InstallBackendSel {
    fn to_kind(self) -> SandboxKind {
        match self {
            Self::Native => SandboxKind::Native,
            Self::Lima => SandboxKind::Lima,
            Self::Wsl2 => SandboxKind::Wsl2,
            Self::Podman => SandboxKind::Podman,
        }
    }

    fn default_for_host() -> Self {
        if cfg!(target_os = "macos") { Self::Lima }
        else if cfg!(target_os = "windows") { Self::Wsl2 }
        else { Self::Podman }
    }
}

#[derive(Debug, clap::Args)]
pub struct InstallArgs {
    /// Claw product id (`openclaw`, `hermes`, ...). Required.
    pub claw: String,
    /// Instance name (must be unique; defaults to "default").
    #[arg(long)]
    pub name: Option<String>,
    /// Sandbox backend. Default = lima on macOS, wsl2 on Windows,
    /// podman on Linux. Pass `native` to skip the VM and install onto
    /// host node/git under `~/.clawenv/`.
    #[arg(long, value_enum)]
    pub backend: Option<InstallBackendSel>,
    /// Specific claw version, or "latest".
    #[arg(long, default_value = "latest")]
    pub version: String,
    /// Host port the sandbox gateway will be exposed at.
    #[arg(long, default_value_t = 3000)]
    pub port: u16,
    #[arg(long, default_value_t = 2)]
    pub cpus: u32,
    #[arg(long, default_value_t = 2048)]
    pub memory_mb: u32,
    /// Install Chromium + VNC bundle (for browser-automation claws).
    #[arg(long)]
    pub browser: bool,
    /// Optional host-side workspace directory to mount/expose into the
    /// instance. Defaults to none.
    #[arg(long)]
    pub workspace: Option<std::path::PathBuf>,
    /// Override the proxy for this install only (does not modify
    /// `~/.clawenv/config.toml`). Format: `http://[user:pass@]host:port`.
    #[arg(long)]
    pub proxy_url: Option<String>,
    /// On --backend native, automatically install missing node/git
    /// into `~/.clawenv/{node,git}` before installing the claw.
    #[arg(long)]
    pub autoinstall_deps: bool,
    /// Render templates + describe what would happen, then stop
    /// before actually invoking limactl/wsl/podman. No side effects.
    /// Intended for CI smoke tests and Gate-0 verification.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run_install(ctx: &Ctx, args: InstallArgs) -> anyhow::Result<()> {
    let backend = args.backend.unwrap_or_else(InstallBackendSel::default_for_host);
    let name = args.name.unwrap_or_else(|| ctx.instance.clone());

    // Pull proxy + mirrors from config.toml when present, then layer
    // --proxy-url on top so a CLI override beats the file. Env vars
    // still override at runtime via the Scope::Installer resolver.
    let mut global = clawops_core::config_loader::load_global()
        .unwrap_or_default();
    if let Some(url) = &args.proxy_url {
        global.proxy.enabled = true;
        global.proxy.http_proxy = url.clone();
        global.proxy.https_proxy = url.clone();
    }
    let proxy = resolve_install_proxy(&global.proxy, backend.to_kind()).await;

    let opts = InstallOpts {
        name: name.clone(),
        claw: args.claw.clone(),
        backend: backend.to_kind(),
        claw_version: args.version,
        gateway_port: args.port,
        cpu_cores: args.cpus,
        memory_mb: args.memory_mb,
        install_browser: args.browser,
        workspace_dir: args.workspace,
        proxy,
        mirrors: global.mirrors,
        dry_run: args.dry_run,
        autoinstall_native_deps: args.autoinstall_deps,
    };

    // Wire a channel so progress streams to stdout as the pipeline runs.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressEvent>(32);
    let sink = ProgressSink::new(tx);

    // Drain progress LIVE — emit each event as it lands rather than
    // buffering for a final blob. In JSON mode this becomes a
    // line-delimited CliEvent::Progress stream the GUI reads in
    // real time; in human mode it's the same `[pct%] stage msg` lines.
    let printer = {
        let output = ctx.output.clone();
        let no_progress = ctx.no_progress;
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if !no_progress { emit_progress(&output, &ev); }
            }
        })
    };

    let o = InstanceOrchestrator::new();
    let report = o.install(opts, sink).await?;
    // Wait for the printer to drain — sink is dropped now, channel closes.
    let _ = printer.await;

    // Final result as a Data event in JSON mode; pretty summary otherwise.
    let summary = serde_json::json!({
        "instance": report.instance,
        "version_output": report.version_output,
        "install_elapsed_secs": report.install_elapsed_secs,
    });
    if ctx.json {
        ctx.output.emit(crate::output::CliEvent::Data { data: summary });
    } else {
        println!();
        println!("✓ Installed {} @ {} ({}s)",
            report.instance.claw,
            report.version_output,
            report.install_elapsed_secs,
        );
        println!("  instance: {}", report.instance.name);
        println!("  backend : {}", report.instance.backend.as_str());
    }
    Ok(())
}

/// Emit a single progress tick — JSON `Progress` event in machine mode,
/// `[pct%] stage msg` line in human mode. Shared by install and upgrade.
fn emit_progress(output: &crate::output::Output, ev: &ProgressEvent) {
    if output.json() {
        output.emit(crate::output::CliEvent::Progress {
            stage: ev.stage.clone(),
            percent: ev.percent.unwrap_or(0),
            message: ev.message.clone(),
        });
    } else {
        let pct = ev.percent.map(|p| format!("{p:>3}%")).unwrap_or_else(|| " … ".into());
        println!("[{pct}] {:<18} {}", ev.stage, ev.message);
    }
}

// ——— upgrade ———

#[derive(Debug, clap::Args)]
pub struct UpgradeArgs {
    /// Instance name to upgrade (falls back to --instance / "default").
    pub name: Option<String>,
    /// Target version; "latest" resolves to the package manager's
    /// upstream default.
    #[arg(long, default_value = "latest")]
    pub to: String,
    /// Don't actually upgrade — only probe the registry for what
    /// the latest available version is. Returns a structured report
    /// with current+latest+has_upgrade. v1's `update-check` behaviour.
    #[arg(long)]
    pub check: bool,
}

pub async fn run_upgrade(ctx: &Ctx, args: UpgradeArgs) -> anyhow::Result<()> {
    let name = args.name.unwrap_or_else(|| ctx.instance.clone());

    // Check-only path: probe registry for latest, report, exit. No VM touch.
    if args.check {
        return run_upgrade_check(ctx, &name).await;
    }

    let opts = UpgradeOpts { name: name.clone(), to_version: args.to };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressEvent>(32);
    let sink = ProgressSink::new(tx);

    // Stream progress live via the same emit_progress helper as install.
    let printer = {
        let output = ctx.output.clone();
        let no_progress = ctx.no_progress;
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                if !no_progress { emit_progress(&output, &ev); }
            }
        })
    };

    let o = InstanceOrchestrator::new();
    let report = o.upgrade(opts, sink).await?;
    let _ = printer.await;

    let summary = serde_json::json!({
        "instance": report.instance,
        "previous_version": report.previous_version,
        "new_version": report.new_version,
        "upgrade_elapsed_secs": report.upgrade_elapsed_secs,
    });
    if ctx.json {
        ctx.output.emit(crate::output::CliEvent::Data { data: summary });
    } else {
        println!();
        println!("✓ Upgraded {} ({}s)",
            report.instance.claw, report.upgrade_elapsed_secs);
        if let Some(prev) = &report.previous_version {
            println!("  {} → {}", prev.trim(), report.new_version.trim());
        } else {
            println!("  (previous version not probeable) → {}",
                report.new_version.trim());
        }
        println!("  instance: {}", report.instance.name);
    }
    Ok(())
}

/// `clawcli upgrade <name> --check` — registry-probe-only flow.
async fn run_upgrade_check(ctx: &Ctx, name: &str) -> anyhow::Result<()> {
    use clawops_core::claw_ops::{provisioning_for, PackageManager};
    use clawops_core::update;

    // Look up instance to get claw id + currently installed version (if VM up).
    let cfg = resolve_instance(name).await?;
    let provisioning = provisioning_for(&cfg.claw)
        .ok_or_else(|| anyhow::anyhow!("unknown claw `{}` (registry mismatch?)", cfg.claw))?;

    // Try to read current version: if backend is sandboxed and VM is up,
    // exec `<bin> --version` inside; if not (or fails), report "(unknown)".
    let current = if let Some(b) = backend_arc(&cfg) {
        b.exec_argv(&["sh", "-c", &provisioning.version_check_cmd()])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "(unknown)".into())
    } else {
        "(unknown)".into()
    };

    // Probe latest from the right registry per package manager.
    let info = match provisioning.package_manager() {
        PackageManager::Npm => {
            update::check_latest_npm(&current, "", provisioning.cli_binary()).await?
        }
        PackageManager::Pip => {
            update::check_latest_pypi(&current, provisioning.cli_binary()).await?
        }
        PackageManager::GitPip { repo, .. } => {
            update::check_latest_github(&current, &repo).await?
        }
    };

    ctx.emit_pretty(&info, |i| {
        println!("Instance     : {}", name);
        println!("Claw         : {}", cfg.claw);
        println!("Current      : {}", i.current);
        println!("Latest       : {}", i.latest);
        println!("Has upgrade  : {}", if i.has_upgrade { "yes" } else { "no" });
        if i.is_security_release {
            println!("⚠ Security release");
        }
        if !i.changelog.is_empty() {
            println!("\nChangelog (preview):\n{}", i.changelog.lines().take(5).collect::<Vec<_>>().join("\n"));
        }
    })?;
    Ok(())
}

// ——— launch (start gateway daemon + dashboard) ———

#[derive(Debug, clap::Args)]
pub struct LaunchArgs {
    /// Instance name to bring online (positional; falls back to ctx.instance).
    pub name: Option<String>,
    /// Cap on how long to wait for the ready_port to respond. Currently
    /// reserved — the orchestrator bakes 120s today (per the v0.3 P3-c
    /// fix). When `InstanceOrchestrator::launch` learns to take a probe
    /// budget, plumb this through.
    #[arg(long, default_value_t = 120)]
    pub probe_secs: u64,
    /// Skip the ready_port probe entirely. Returns once the daemons are
    /// spawned, without verifying HTTP responsiveness. Use for cases
    /// where the network probe itself is unreliable (proxied / IPv6-
    /// only / weird routing) but the user has another readiness signal.
    #[arg(long)]
    pub no_probe: bool,
}

pub async fn run_launch(ctx: &Ctx, args: LaunchArgs) -> anyhow::Result<()> {
    let n = resolve_name(args.name, ctx);
    let o = InstanceOrchestrator::new();
    let report = o.launch_with_probe(&n, args.probe_secs, args.no_probe).await?;
    ctx.emit_pretty(&report, |r| {
        if r.started_processes.is_empty() {
            println!("✓ Instance `{}` ready (no auto-start daemons for this claw — interactive only).", r.instance_name);
            return;
        }
        println!("✓ Started {} on instance `{}`", r.started_processes.join(" + "), r.instance_name);
        match r.ready_port {
            Some(p) => println!("  Listening on http://127.0.0.1:{p}"),
            None => println!("  ⚠ Port did not respond within 30s — check logs"),
        }
    })?;
    Ok(())
}

// ——— net-check (host or sandbox preflight) ———

/// Which side of the sandbox boundary to probe from.
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum NetCheckMode {
    /// Run probes from the host process (does this machine have egress?).
    Host,
    /// Run probes FROM INSIDE a sandbox via its in-VM curl. Requires the
    /// instance to be up. Catches cases where host reachability is fine
    /// but the VM's resolv.conf / proxy env / CA bundle is broken.
    Sandbox,
}

pub async fn run_net_check(
    ctx: &Ctx,
    mode: NetCheckMode,
    name: Option<String>,
    proxy_url: Option<String>,
) -> anyhow::Result<()> {
    // proxy_url override: if the caller wants to test connectivity
    // through a specific proxy (e.g. a corporate one not yet in
    // config.toml), set the standard env vars before the probe runs.
    // The preflight code reads HTTPS_PROXY/HTTP_PROXY directly.
    let _proxy_guard = proxy_url.as_ref().map(|url| {
        // Returning a guard means the env vars unset on drop. Avoids
        // bleeding into the rest of the process (a JSON-mode caller
        // that runs net-check then status would otherwise inherit them).
        struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                for (k, v) in self.0.drain(..) {
                    match v {
                        Some(prev) => std::env::set_var(k, prev),
                        None => std::env::remove_var(k),
                    }
                }
            }
        }
        let saved: Vec<(&'static str, Option<std::ffi::OsString>)> = ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"]
            .iter()
            .map(|k| (*k, std::env::var_os(k)))
            .collect();
        for k in ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"] {
            std::env::set_var(k, url);
        }
        EnvGuard(saved)
    });
    let rep = match mode {
        NetCheckMode::Host => preflight::run_preflight().await?,
        NetCheckMode::Sandbox => {
            let n = resolve_name(name, ctx);
            let cfg = resolve_instance(&n).await?;
            let backend = backend_arc(&cfg).ok_or_else(|| {
                anyhow::anyhow!("instance `{n}` is native (no sandbox to probe from)")
            })?;
            preflight::run_sandbox_preflight(&backend).await?
        }
    };
    ctx.emit_pretty(&rep, |r| {
        let mut t = new_table(["host", "reachable", "status", "latency", "error"]);
        for h in &r.hosts {
            t.add_row([
                h.host.clone(),
                if h.reachable { "yes".into() } else { "no".into() },
                h.http_status.map_or("—".into(), |s| s.to_string()),
                h.latency_ms.map_or("—".into(), |ms| format!("{ms} ms")),
                h.error.clone().unwrap_or_else(|| "—".into()),
            ]);
        }
        println!("{t}");
        if let Some(s) = &r.suggestion { println!("\n{s}"); }
    })?;
    if !rep.all_reachable {
        std::process::exit(1);
    }
    Ok(())
}

// ——— doctor (aggregate) ———

#[derive(Serialize)]
struct CompositeDoctor {
    name: String,
    native: clawops_core::native_ops::NativeDoctorReport,
    sandbox: Option<SandboxDoctorReport>,
    download: clawops_core::download_ops::DownloadDoctorReport,
    overall_healthy: bool,
}

pub async fn run_doctor(
    ctx: &Ctx,
    name: Option<String>,
    all: bool,
    fix: bool,
) -> anyhow::Result<()> {
    if all {
        return run_doctor_all(ctx).await;
    }
    let n = resolve_name(name, ctx);
    let cfg = resolve_instance(&n).await?;

    let native = DefaultNativeOps::new().doctor().await?;
    let mut sandbox = match sandbox_ops_for(&cfg) {
        Some(ops) => Some(ops.doctor().await?),
        None => None,
    };

    // --fix: collect the issue ids each layer reports, then ask the
    // sandbox layer to repair them. We re-doctor afterward so the
    // returned report reflects the post-repair state, not the
    // pre-repair snapshot.
    if fix {
        if let (Some(rep), Some(ops)) = (sandbox.as_ref(), sandbox_ops_for(&cfg)) {
            let issue_ids: Vec<String> = rep.issues.iter().map(|i| i.id.clone()).collect();
            if !issue_ids.is_empty() {
                ctx.emit_text(format!(
                    "doctor: attempting repair of {} issue(s): {}",
                    issue_ids.len(), issue_ids.join(", "),
                ));
                if let Err(e) = ops.repair(&issue_ids, ProgressSink::noop()).await {
                    ctx.emit_text(format!("doctor: repair returned error: {e} (continuing)"));
                }
                // Re-doctor so the report reflects the post-repair state.
                sandbox = Some(ops.doctor().await?);
            }
        }
    }

    let download = CatalogBackedDownloadOps::with_defaults().doctor().await?;

    let overall_healthy = native.healthy()
        && sandbox.as_ref().is_none_or(|s| s.healthy())
        && download.healthy();

    let report = CompositeDoctor {
        name: n.clone(),
        native,
        sandbox,
        download,
        overall_healthy,
    };
    ctx.emit_pretty(&report, |r| {
        println!("=== doctor: {} ===", r.name);
        print_native(&r.native);
        if let Some(s) = &r.sandbox { print_sandbox(s); }
        print_download(&r.download);
        println!();
        if r.overall_healthy {
            println!("Overall: healthy");
        } else {
            println!("Overall: unhealthy — see issues above");
        }
    })?;
    Ok(())
}

/// `clawcli doctor --all` — diagnose every registered instance.
/// Returns Vec<CompositeDoctor>; per-instance failures are captured in
/// the report rather than aborting (so one broken VM doesn't hide the
/// rest's state from the user).
async fn run_doctor_all(ctx: &Ctx) -> anyhow::Result<()> {
    let reg = InstanceRegistry::with_default_path();
    let instances = reg.list().await?;
    let mut reports: Vec<CompositeDoctor> = Vec::with_capacity(instances.len());
    for inst in instances {
        let native = DefaultNativeOps::new().doctor().await?;
        let sandbox = match sandbox_ops_for(&inst) {
            Some(ops) => ops.doctor().await.ok(),
            None => None,
        };
        let download = CatalogBackedDownloadOps::with_defaults().doctor().await?;
        let overall_healthy = native.healthy()
            && sandbox.as_ref().is_none_or(|s| s.healthy())
            && download.healthy();
        reports.push(CompositeDoctor {
            name: inst.name.clone(),
            native,
            sandbox,
            download,
            overall_healthy,
        });
    }
    ctx.emit_pretty(&reports, |rs| {
        for r in rs {
            println!("=== doctor: {} ===", r.name);
            print_native(&r.native);
            if let Some(s) = &r.sandbox { print_sandbox(s); }
            print_download(&r.download);
            println!("Overall: {}",
                if r.overall_healthy { "healthy" } else { "unhealthy" });
            println!();
        }
    })?;
    Ok(())
}

fn print_native(r: &clawops_core::native_ops::NativeDoctorReport) {
    println!("[native]");
    if r.issues.is_empty() { println!("  ok"); return; }
    for i in &r.issues {
        let sev = format!("{:?}", i.severity);
        println!("  [{}] {} — {}", severity_color(&sev), i.id, i.message);
    }
}
fn print_sandbox(r: &SandboxDoctorReport) {
    println!("[sandbox:{}]", format!("{:?}", r.backend).to_lowercase());
    if r.issues.is_empty() { println!("  ok"); return; }
    for i in &r.issues {
        let sev = format!("{:?}", i.severity);
        println!("  [{}] {} — {}", severity_color(&sev), i.id, i.message);
    }
}
fn print_download(r: &clawops_core::download_ops::DownloadDoctorReport) {
    println!("[download]");
    if r.issues.is_empty() { println!("  ok"); return; }
    for i in &r.issues {
        let sev = format!("{:?}", i.severity);
        println!("  [{}] {} — {}", severity_color(&sev), i.id, i.message);
    }
}

// ——— export / import (bundle distribution) ———

#[derive(Debug, clap::Args)]
pub struct ExportArgs {
    /// Instance name to export (positional; falls back to ctx.instance).
    pub name: Option<String>,
    /// Output path for the bundle. `.tar.gz` is conventional but not enforced.
    #[arg(long)]
    pub output: std::path::PathBuf,
}

#[derive(Debug, clap::Args)]
pub struct ImportArgs {
    /// Path to the bundle file produced by `clawcli export`.
    #[arg(long)]
    pub file: std::path::PathBuf,
    /// Instance name to register the imported VM/container under.
    #[arg(long)]
    pub name: String,
    /// Host port to expose the gateway on (matches v1 semantics).
    #[arg(long, default_value_t = 3000)]
    pub port: u16,
}

pub async fn run_export(ctx: &Ctx, args: ExportArgs) -> anyhow::Result<()> {
    use clawops_core::claw_ops::provisioning_for;
    use clawops_core::export::{BundleManifest, MANIFEST_FILENAME};

    let name = resolve_name(args.name, ctx);
    let cfg = resolve_instance(&name).await?;
    if matches!(cfg.backend, SandboxKind::Native) {
        anyhow::bail!("export of native instances is not supported (no VM image to capture)");
    }
    let backend = backend_arc(&cfg).ok_or_else(|| {
        anyhow::anyhow!("instance `{name}` has no sandbox backend to export")
    })?;

    // Best-effort current claw version probe (informational; bundle is
    // still importable without it). VM may be stopped — tolerate.
    let claw_version = if let Some(p) = provisioning_for(&cfg.claw) {
        backend
            .exec_argv(&["sh", "-c", &p.version_check_cmd()])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    // The export composes a 2-entry tar.gz at args.output:
    //   - clawenv-bundle.toml   (manifest)
    //   - payload.tar           (whatever backend.export_image produces;
    //                            Lima emits a tar.gz, Podman a tar, etc.
    //                            Importer reads sandbox_type from manifest
    //                            and feeds payload to the right backend).
    let work = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("create export work dir: {e}"))?;
    let payload_path = work.path().join(BundleManifest::INNER_PAYLOAD_FILENAME);

    ctx.emit_text("stopping VM (export of running VM risks torn snapshot)");
    let _ = sandbox_ops_for(&cfg).map(|ops| async move {
        let _ = ops.stop(ProgressSink::noop(), CancellationToken::new()).await;
    });

    ctx.emit_text(format!(
        "exporting backend image → {}",
        payload_path.display()
    ));
    backend.export_image(&payload_path).await
        .map_err(|e| anyhow::anyhow!("backend export_image: {e}"))?;

    let manifest = BundleManifest::build(
        &cfg.claw,
        &claw_version,
        cfg.backend.as_str(),
    );
    manifest.write_to_dir(work.path())
        .map_err(|e| anyhow::anyhow!("write manifest: {e}"))?;

    ctx.emit_text(format!("wrapping with manifest → {}", args.output.display()));
    // tar czf <output> -C <work> manifest payload
    let work_str = work.path().to_string_lossy().to_string();
    let out_str = args.output.to_string_lossy().to_string();
    let status = tokio::process::Command::new("tar")
        .args([
            "czf", &out_str,
            "-C", &work_str,
            MANIFEST_FILENAME,
            BundleManifest::INNER_PAYLOAD_FILENAME,
        ])
        .status()
        .await
        .map_err(|e| anyhow::anyhow!("spawn tar (wrap): {e}"))?;
    if !status.success() {
        anyhow::bail!("tar wrap exited with {:?}", status.code());
    }

    let summary = serde_json::json!({
        "instance": cfg.name,
        "claw": cfg.claw,
        "claw_version": claw_version,
        "backend": cfg.backend.as_str(),
        "output": args.output,
    });
    if ctx.json {
        ctx.output.emit(crate::output::CliEvent::Data { data: summary });
    } else {
        println!("✓ Exported {} → {}", cfg.name, args.output.display());
    }
    Ok(())
}

pub async fn run_import(ctx: &Ctx, args: ImportArgs) -> anyhow::Result<()> {
    use clawops_core::export::BundleManifest;
    use clawops_core::instance::{InstanceConfig, InstanceRegistry, PortBinding};
    use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};
    use std::sync::Arc;

    if !args.file.exists() {
        anyhow::bail!("bundle not found: {}", args.file.display());
    }

    // Peek manifest first — fail fast if the bundle is malformed before
    // we touch the VM/container layer.
    ctx.emit_text(format!("inspecting manifest in {}", args.file.display()));
    let manifest = BundleManifest::peek_from_tarball(&args.file).await?;

    let kind = SandboxKind::parse(&manifest.sandbox_type)
        .ok_or_else(|| anyhow::anyhow!(
            "manifest sandbox_type `{}` does not match any v2 backend (expected one of native/lima/wsl2/podman). \
             Pre-v2 bundles need a compat shim — open an issue if you need this.",
            manifest.sandbox_type,
        ))?;

    if matches!(kind, SandboxKind::Native) {
        anyhow::bail!("import of native bundles is not supported (no VM image to restore)");
    }

    // Extract inner payload to a scratch path the backend can read.
    let scratch = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("create import work dir: {e}"))?;
    let payload = scratch.path().join(BundleManifest::INNER_PAYLOAD_FILENAME);
    ctx.emit_text(format!("extracting payload → {}", payload.display()));
    BundleManifest::extract_inner_payload(&args.file, &payload).await?;

    let backend: Arc<dyn SandboxBackend> = match kind {
        SandboxKind::Lima => Arc::new(LimaBackend::new(&args.name)),
        SandboxKind::Wsl2 => Arc::new(WslBackend::new(&args.name)),
        SandboxKind::Podman => Arc::new(PodmanBackend::new(&args.name)),
        SandboxKind::Native => unreachable!("native bailed above"),
    };

    ctx.emit_text("importing image into backend");
    backend.import_image(&payload).await
        .map_err(|e| anyhow::anyhow!("backend import_image: {e}"))?;

    // Insert registry record so subsequent `clawcli list` / `start` / `launch`
    // can find this instance. created_at + updated_at use bundle import time
    // (the bundle was made earlier, but the local record reflects when it
    // was registered here).
    let now = chrono::Utc::now().to_rfc3339();
    let cfg = InstanceConfig {
        name: args.name.clone(),
        claw: manifest.claw_type.clone(),
        backend: kind,
        sandbox_instance: args.name.clone(),
        ports: vec![PortBinding {
            host: args.port,
            guest: 3000,
            label: "gateway".into(),
        }],
        created_at: now.clone(),
        updated_at: now,
        note: format!(
            "imported from {} (originally clawenv {} on {})",
            args.file.display(),
            manifest.clawenv_version,
            manifest.source_platform,
        ),
    };
    let reg = InstanceRegistry::with_default_path();
    reg.insert(cfg.clone()).await
        .map_err(|e| anyhow::anyhow!("registry insert: {e}"))?;

    let summary = serde_json::json!({
        "instance": cfg,
        "manifest": {
            "schema_version": manifest.schema_version,
            "clawenv_version": manifest.clawenv_version,
            "claw_type": manifest.claw_type,
            "claw_version": manifest.claw_version,
            "sandbox_type": manifest.sandbox_type,
            "source_platform": manifest.source_platform,
            "created_at": manifest.created_at,
        },
    });
    if ctx.json {
        ctx.output.emit(crate::output::CliEvent::Data { data: summary });
    } else {
        println!("✓ Imported {} → instance `{}` ({})",
            args.file.display(), args.name, kind.as_str());
        println!("  claw         : {} {}", manifest.claw_type,
            if manifest.claw_version.is_empty() { "—".into() } else { manifest.claw_version.clone() });
        println!("  source       : clawenv {} ({})",
            manifest.clawenv_version, manifest.source_platform);
        println!("  next         : `clawcli start {}` to bring the VM online", args.name);
    }
    Ok(())
}

// ——— tests ———

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_sanity() {
        let b = default_backend_for_host();
        assert!(matches!(
            b,
            SandboxKind::Lima | SandboxKind::Wsl2 | SandboxKind::Podman
        ));
    }

    #[test]
    fn resolve_name_prefers_positional() {
        let ctx = Ctx {
            json: false, quiet: false, no_progress: false,
            instance: "fallback".into(),
            output: crate::output::Output::new(false),
        };
        assert_eq!(resolve_name(Some("mine".into()), &ctx), "mine");
        assert_eq!(resolve_name(None, &ctx), "fallback");
    }
}
