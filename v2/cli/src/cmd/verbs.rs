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
use clawops_core::instance::{InstanceConfig, InstanceRegistry, SandboxKind};
use clawops_core::native_ops::{DefaultNativeOps, NativeOps};
use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};
use clawops_core::sandbox_ops::{
    LimaOps, PodmanOps, SandboxDoctorReport, SandboxOps, WslOps,
};
use clawops_core::{CancellationToken, ProgressSink};
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
