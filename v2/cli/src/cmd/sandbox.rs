use clap::Subcommand;
use clawops_core::sandbox_ops::{BackendKind, LimaOps, PodmanOps, SandboxOps, WslOps};
use clawops_core::{CancellationToken, ProgressSink};

use crate::shared::{new_table, severity_color, Ctx};

#[derive(Subcommand)]
pub enum SandboxCmd {
    /// Show VM status + capabilities.
    Status {
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    /// Start the sandbox VM.
    Start { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Stop the sandbox VM.
    Stop { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Restart.
    Restart { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Port forward management.
    Port {
        #[command(subcommand)] op: PortOp,
    },
    /// Run diagnostics.
    Doctor { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Apply repair recipes for given issue IDs.
    Repair {
        /// Issue IDs to repair (e.g. `vm-not-running`).
        issue_ids: Vec<String>,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    /// Show resource usage.
    Stats { #[arg(long, value_enum)] backend: Option<BackendSel> },
}

#[derive(Subcommand)]
pub enum PortOp {
    List {
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    Add {
        host: u16, guest: u16,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    Remove {
        host: u16,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum BackendSel { Lima, Wsl2, Podman }

fn pick_default_backend() -> BackendSel {
    if cfg!(target_os = "macos") { BackendSel::Lima }
    else if cfg!(target_os = "windows") { BackendSel::Wsl2 }
    else { BackendSel::Podman }
}

fn ops_for(sel: BackendSel, instance: &str) -> Box<dyn SandboxOps> {
    match sel {
        BackendSel::Lima => Box::new(LimaOps::new(instance)),
        BackendSel::Wsl2 => Box::new(WslOps::new(instance)),
        BackendSel::Podman => Box::new(PodmanOps::new(instance)),
    }
}

fn resolve(backend: Option<BackendSel>) -> BackendSel {
    backend.unwrap_or_else(pick_default_backend)
}

pub async fn run(cmd: SandboxCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        SandboxCmd::Status { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            let s = ops.status().await?;
            ctx.emit_pretty(&s, |st| {
                println!("Backend      : {}", format!("{:?}", st.backend).to_lowercase());
                println!("Instance     : {}", st.instance_name);
                println!("State        : {}", format!("{:?}", st.state).to_lowercase());
                if let Some(c) = st.cpu_cores { println!("CPU cores    : {c}"); }
                if let Some(m) = st.memory_mb { println!("Memory       : {m} MB"); }
                if let Some(d) = st.disk_gb { println!("Disk         : {d} GB"); }
                if let Some(ip) = &st.ip { println!("IP           : {ip}"); }
            })?;
        }
        SandboxCmd::Start { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            ops.start(ProgressSink::noop(), CancellationToken::new()).await?;
            ctx.emit_text("started");
        }
        SandboxCmd::Stop { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            ops.stop(ProgressSink::noop(), CancellationToken::new()).await?;
            ctx.emit_text("stopped");
        }
        SandboxCmd::Restart { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            ops.restart(ProgressSink::noop(), CancellationToken::new()).await?;
            ctx.emit_text("restarted");
        }
        SandboxCmd::Port { op } => {
            match op {
                PortOp::List { backend } => {
                    let ops = ops_for(resolve(backend), &ctx.instance);
                    let ports = ops.list_ports().await?;
                    ctx.emit_pretty(&ports, |rows| {
                        if rows.is_empty() {
                            println!("No port forwards configured.");
                        } else {
                            let mut t = new_table(["host", "guest", "native_id"]);
                            for p in rows {
                                t.add_row([
                                    p.host.to_string(),
                                    p.guest.to_string(),
                                    p.native_id.clone().unwrap_or_else(|| "—".into()),
                                ]);
                            }
                            println!("{t}");
                        }
                    })?;
                }
                PortOp::Add { host, guest, backend } => {
                    let ops = ops_for(resolve(backend), &ctx.instance);
                    ops.add_port(host, guest).await?;
                    ctx.emit_text(format!("added {host} → {guest}"));
                }
                PortOp::Remove { host, backend } => {
                    let ops = ops_for(resolve(backend), &ctx.instance);
                    ops.remove_port(host).await?;
                    ctx.emit_text(format!("removed {host}"));
                }
            }
        }
        SandboxCmd::Doctor { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            let r = ops.doctor().await?;
            ctx.emit_pretty(&r, |rep| {
                println!(
                    "backend={} instance={} issues={}",
                    format!("{:?}", rep.backend).to_lowercase(),
                    rep.instance_name,
                    rep.issues.len(),
                );
                if rep.issues.is_empty() {
                    println!("No issues found.");
                } else {
                    for i in &rep.issues {
                        let sev = format!("{:?}", i.severity);
                        println!("[{}] {} — {}", severity_color(&sev), i.id, i.message);
                        if let Some(hint) = &i.repair_hint {
                            println!("    hint: {hint}");
                        }
                    }
                }
                println!("\nchecked at {}", rep.checked_at);
            })?;
        }
        SandboxCmd::Repair { issue_ids, backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            ops.repair(&issue_ids, ProgressSink::noop()).await?;
            ctx.emit_text(format!("repaired {} issue(s)", issue_ids.len()));
        }
        SandboxCmd::Stats { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            let s = ops.stats().await?;
            ctx.emit(&s)?;
        }
    }
    Ok(())
}

// Suppress unused import warning for BackendKind — kept in re-exports so
// external tests can reference it; no direct use in this file.
#[allow(dead_code)]
const _: Option<BackendKind> = None;
