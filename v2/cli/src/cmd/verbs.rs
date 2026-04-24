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

/// Aggregate view for `clawcli status [<name>]`. Combines instance
/// registry data, the sandbox VM's runtime state, and (if the instance
/// is registered) its associated claw.
#[derive(Serialize)]
struct StatusView {
    name: String,
    claw: String,
    backend: String,
    registered: bool,
    vm: Option<clawops_core::sandbox_ops::SandboxStatus>,
    ports: Vec<clawops_core::instance::PortBinding>,
}

pub async fn run_status(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    let n = resolve_name(name, ctx);
    let reg = InstanceRegistry::with_default_path();
    let found = reg.find(&n).await?;
    let registered = found.is_some();
    let cfg = match found {
        Some(c) => c,
        None => resolve_instance(&n).await?,
    };

    let vm = match sandbox_ops_for(&cfg) {
        Some(ops) => Some(ops.status().await?),
        None => None,
    };
    let view = StatusView {
        name: cfg.name.clone(),
        claw: cfg.claw.clone(),
        backend: cfg.backend.as_str().to_string(),
        registered,
        vm,
        ports: cfg.ports.clone(),
    };
    ctx.emit_pretty(&view, |v| {
        println!("Instance  : {}", v.name);
        println!("Claw      : {}", if v.claw.is_empty() { "—" } else { v.claw.as_str() });
        println!("Backend   : {}", v.backend);
        println!("Registered: {}", if v.registered { "yes" } else { "no (synthesised view)" });
        if let Some(st) = &v.vm {
            println!("VM state  : {}", format!("{:?}", st.state).to_lowercase());
        } else {
            println!("VM state  : n/a (native)");
        }
        if !v.ports.is_empty() {
            let mut t = new_table(["host", "guest", "label"]);
            for p in &v.ports {
                t.add_row([p.host.to_string(), p.guest.to_string(),
                    if p.label.is_empty() { "—".into() } else { p.label.clone() }]);
            }
            println!("{t}");
        }
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
    ctx.emit_text(format!("{verb}ed {n}"));
    Ok(())
}

pub async fn run_start(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    lifecycle_dispatch(ctx, name, "start").await
}
pub async fn run_stop(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    lifecycle_dispatch(ctx, name, "stop").await
}
pub async fn run_restart(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    lifecycle_dispatch(ctx, name, "restart").await
}

// ——— list (alias for `instance list`) ———

pub async fn run_list(ctx: &Ctx) -> anyhow::Result<()> {
    let reg = InstanceRegistry::with_default_path();
    let instances = reg.list().await?;
    ctx.emit_pretty(&instances, |rows| {
        if rows.is_empty() {
            println!("No instances registered. Use `clawcli install <claw>` to create one.");
            return;
        }
        let mut t = new_table(["name", "claw", "backend", "sandbox-instance", "ports"]);
        for i in rows {
            let ports: Vec<String> =
                i.ports.iter().map(|p| format!("{}→{}", p.host, p.guest)).collect();
            t.add_row([
                i.name.clone(),
                if i.claw.is_empty() { "—".into() } else { i.claw.clone() },
                i.backend.as_str().into(),
                if i.sandbox_instance.is_empty() {
                    "—".into()
                } else {
                    i.sandbox_instance.clone()
                },
                if ports.is_empty() { "—".into() } else { ports.join(", ") },
            ]);
        }
        println!("{t}");
    })?;
    Ok(())
}

// ——— exec ———

/// Serializable shape for `--json` output.
#[derive(Serialize, Debug)]
struct ExecReport {
    instance: String,
    stdout: String,
}

pub async fn run_exec(
    ctx: &Ctx,
    name: Option<String>,
    cmd: Vec<String>,
) -> anyhow::Result<()> {
    if cmd.is_empty() {
        anyhow::bail!("exec: no command given (use `clawcli exec <name> -- cmd args...`)");
    }
    let n = resolve_name(name, ctx);
    let cfg = resolve_instance(&n).await?;
    let backend = backend_arc(&cfg).ok_or_else(|| {
        anyhow::anyhow!("instance `{n}` is native (no VM to exec into)")
    })?;
    let argv_refs: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
    let stdout = backend
        .exec_argv(&argv_refs)
        .await
        .map_err(|e| anyhow::anyhow!("exec in {n}: {e}"))?;
    if ctx.json {
        ctx.emit(&ExecReport { instance: n, stdout })?;
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

/// Backend selector for `clawcli install`. Mirrors SandboxKind but
/// only exposes the 3 sandboxed backends — Native is a separate code
/// path (R3.1 — deferred).
#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum InstallBackendSel {
    Lima,
    Wsl2,
    Podman,
}

impl InstallBackendSel {
    fn to_kind(self) -> SandboxKind {
        match self {
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
    /// Claw product id (`openclaw`, `hermes`, ...).
    pub claw: String,
    /// Instance name (must be unique; defaults to "default").
    #[arg(long)]
    pub name: Option<String>,
    /// Sandbox backend. Default = lima on macOS, wsl2 on Windows,
    /// podman on Linux.
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
    pub install_browser: bool,
    /// Render templates + describe what would happen, then stop
    /// before actually invoking limactl/wsl/podman. No side effects.
    /// Intended for CI smoke tests and Gate-0 verification.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run_install(ctx: &Ctx, args: InstallArgs) -> anyhow::Result<()> {
    let name = args.name.unwrap_or_else(|| ctx.instance.clone());
    let backend = args.backend.unwrap_or_else(InstallBackendSel::default_for_host);

    // Auto-pull proxy + mirrors from v1's config.toml when present —
    // users expect `clawcli install` to honor their existing proxy
    // setup without having to re-specify it. Env vars still override
    // via the Scope::Installer resolver at runtime.
    let v1_config = clawops_core::config_loader::load_global()
        .unwrap_or_default();
    let proxy = resolve_install_proxy(&v1_config.proxy, backend.to_kind()).await;

    let opts = InstallOpts {
        name: name.clone(),
        claw: args.claw.clone(),
        backend: backend.to_kind(),
        claw_version: args.version,
        gateway_port: args.port,
        cpu_cores: args.cpus,
        memory_mb: args.memory_mb,
        install_browser: args.install_browser,
        workspace_dir: None,
        proxy,
        mirrors: v1_config.mirrors,
        dry_run: args.dry_run,
    };

    // Wire a channel so we can stream progress to stdout as the pipeline runs.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressEvent>(32);
    let sink = ProgressSink::new(tx);
    let json = ctx.json;

    // Spawn a drainer so the pipeline doesn't block on a full channel.
    // In JSON mode we collect events; in pretty mode we print them live.
    let printer = tokio::spawn(async move {
        let mut events: Vec<ProgressEvent> = Vec::new();
        while let Some(ev) = rx.recv().await {
            if !json {
                let pct = ev.percent.map(|p| format!("{p:>3}%")).unwrap_or_else(|| " … ".into());
                println!("[{pct}] {:<18} {}", ev.stage, ev.message);
            }
            events.push(ev);
        }
        events
    });

    let o = InstanceOrchestrator::new();
    let report = o.install(opts, sink).await?;
    let events = printer.await.unwrap_or_default();

    if json {
        // ProgressEvent is currently non-Serialize; hand-shape it here.
        let event_rows: Vec<serde_json::Value> = events.iter().map(|e| serde_json::json!({
            "percent": e.percent,
            "stage": e.stage,
            "message": e.message,
        })).collect();
        let blob = serde_json::json!({
            "instance": report.instance,
            "version_output": report.version_output,
            "install_elapsed_secs": report.install_elapsed_secs,
            "events": event_rows,
        });
        println!("{}", serde_json::to_string_pretty(&blob)?);
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

// ——— upgrade ———

#[derive(Debug, clap::Args)]
pub struct UpgradeArgs {
    /// Instance name to upgrade (falls back to --instance / "default").
    pub name: Option<String>,
    /// Target version; "latest" resolves to the package manager's
    /// upstream default.
    #[arg(long, default_value = "latest")]
    pub to: String,
}

pub async fn run_upgrade(ctx: &Ctx, args: UpgradeArgs) -> anyhow::Result<()> {
    let name = args.name.unwrap_or_else(|| ctx.instance.clone());
    let opts = UpgradeOpts { name: name.clone(), to_version: args.to };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressEvent>(32);
    let sink = ProgressSink::new(tx);
    let json = ctx.json;

    let printer = tokio::spawn(async move {
        let mut events: Vec<ProgressEvent> = Vec::new();
        while let Some(ev) = rx.recv().await {
            if !json {
                let pct = ev.percent.map(|p| format!("{p:>3}%"))
                    .unwrap_or_else(|| " … ".into());
                println!("[{pct}] {:<18} {}", ev.stage, ev.message);
            }
            events.push(ev);
        }
        events
    });

    let o = InstanceOrchestrator::new();
    let report = o.upgrade(opts, sink).await?;
    let events = printer.await.unwrap_or_default();

    if json {
        let event_rows: Vec<serde_json::Value> = events.iter().map(|e| serde_json::json!({
            "percent": e.percent, "stage": e.stage, "message": e.message,
        })).collect();
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "instance": report.instance,
            "previous_version": report.previous_version,
            "new_version": report.new_version,
            "upgrade_elapsed_secs": report.upgrade_elapsed_secs,
            "events": event_rows,
        }))?);
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
) -> anyhow::Result<()> {
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

pub async fn run_doctor(ctx: &Ctx, name: Option<String>) -> anyhow::Result<()> {
    let n = resolve_name(name, ctx);
    let cfg = resolve_instance(&n).await?;

    let native = DefaultNativeOps::new().doctor().await?;
    let sandbox = match sandbox_ops_for(&cfg) {
        Some(ops) => Some(ops.doctor().await?),
        None => None,
    };
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
        let ctx = Ctx { json: false, quiet: false, instance: "fallback".into() };
        assert_eq!(resolve_name(Some("mine".into()), &ctx), "mine");
        assert_eq!(resolve_name(None, &ctx), "fallback");
    }
}
