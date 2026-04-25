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
    /// List ALL VMs/containers known to the host backend (whether
    /// registered with v2 or not). Useful for discovering orphan VMs.
    List { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Rename a sandbox VM. Backend-specific: Lima supports it via
    /// limactl; WSL/Podman need recreate (deferred — bails clean).
    Rename {
        #[arg(long)] from: String,
        #[arg(long)] to: String,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    /// Edit sandbox resource allocation (CPUs, memory). Lima only;
    /// WSL/Podman bail.
    Edit {
        #[arg(long)] cpus: Option<u32>,
        #[arg(long = "memory-mb")] memory_mb: Option<u32>,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
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
        SandboxCmd::List { backend } => {
            // Backend-host inventory: for Lima we shell out to `limactl list`;
            // for Podman `podman ps -a`; for WSL `wsl -l -v`. Each emits a
            // compact JSON-friendly Vec of {name,status} pairs.
            use clawops_core::common::{CommandRunner, CommandSpec, CancellationToken};
            use clawops_core::runners::LocalProcessRunner;
            use std::time::Duration;
            let sel = resolve(backend);
            let runner = LocalProcessRunner::new();
            let (cmd, args): (&str, Vec<&str>) = match sel {
                BackendSel::Lima => ("limactl", vec!["list", "--format", "json"]),
                BackendSel::Podman => ("podman", vec!["ps", "-a", "--format", "{{.Names}}\t{{.Status}}"]),
                BackendSel::Wsl2 => ("wsl", vec!["-l", "-v"]),
            };
            let res = runner.exec(
                CommandSpec::new(cmd, args).with_timeout(Duration::from_secs(5)),
                CancellationToken::new(),
            ).await?;
            let stdout = res.stdout;
            ctx.emit_pretty(&stdout, |raw| {
                println!("{raw}");
            })?;
        }
        SandboxCmd::Rename { from, to, backend } => {
            let _ = (from, to, backend);
            anyhow::bail!(
                "sandbox rename: backend trait does not yet expose rename; \
                 deferred to follow-up. For Lima, use `limactl rename` directly."
            );
        }
        SandboxCmd::Edit { cpus, memory_mb, backend } => {
            // Lima edit: rewrite `cpus:` / `memory:` keys in the lima.yaml.
            // Other backends bail clean. Today we only support Lima.
            let sel = resolve(backend);
            if !matches!(sel, BackendSel::Lima) {
                anyhow::bail!(
                    "sandbox edit currently supports only Lima; for WSL/Podman \
                     destroy + recreate with new resource flags."
                );
            }
            let yaml_path = clawops_core::paths::lima_home()
                .join(&ctx.instance).join("lima.yaml");
            if !yaml_path.exists() {
                anyhow::bail!("lima.yaml not found: {}", yaml_path.display());
            }
            let body = tokio::fs::read_to_string(&yaml_path).await?;
            let body = if let Some(c) = cpus {
                rewrite_yaml_scalar(&body, "cpus", &c.to_string())
            } else { body };
            let body = if let Some(m) = memory_mb {
                rewrite_yaml_scalar(&body, "memory", &format!("\"{m}MiB\""))
            } else { body };
            tokio::fs::write(&yaml_path, body).await?;
            ctx.emit_text(format!(
                "Updated {}; restart the VM with `clawcli restart {}` to apply.",
                yaml_path.display(), ctx.instance
            ));
        }
    }
    Ok(())
}

/// Rewrite a top-level `key: value` line in a YAML-ish file. If the
/// key is missing, append it. v1 lima.yaml mutation pattern (its keys
/// are flat top-level scalars for cpus/memory, so this works).
fn rewrite_yaml_scalar(body: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(body.len() + 32);
    let mut replaced = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&format!("{key}:")) && !replaced {
            out.push_str(&format!("{key}: {value}"));
            out.push('\n');
            replaced = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !replaced {
        out.push_str(&format!("{key}: {value}\n"));
    }
    out
}

// Suppress unused import warning for BackendKind — kept in re-exports so
// external tests can reference it; no direct use in this file.
#[allow(dead_code)]
const _: Option<BackendKind> = None;
