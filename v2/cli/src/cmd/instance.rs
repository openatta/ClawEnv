//! Instance command group — create / destroy / list / info / health.

use clap::Subcommand;
use clawops_core::download_ops::{CatalogBackedDownloadOps, DownloadOps};
use clawops_core::instance::{
    CreateOpts, InstanceOrchestrator, PortBinding, SandboxKind,
};
use clawops_core::native_ops::{DefaultNativeOps, NativeOps};
use clawops_core::sandbox_ops::{LimaOps, PodmanOps, SandboxOps, WslOps};
use clawops_core::ProgressSink;
use serde::Serialize;

use crate::shared::Ctx;

#[derive(Subcommand)]
pub enum InstanceCmd {
    /// List registered instances.
    List,
    /// Show detail for a single instance.
    Info { name: String },
    /// Create a new instance record (preflight + port forwards).
    Create {
        #[arg(long)] name: String,
        #[arg(long)] claw: String,
        #[arg(long, value_enum)] backend: BackendArg,
        /// VM/container instance name (sandboxed backends only).
        #[arg(long, default_value = "default")]
        sandbox_instance: String,
        /// Port forward spec: host:guest[:label], repeatable.
        #[arg(long = "port")]
        ports: Vec<String>,
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long)]
        autoinstall_deps: bool,
    },
    /// Remove an instance record (+ port forwards).
    Destroy { name: String },
    /// Cross-layer composed health check.
    Health,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum BackendArg { Native, Lima, Wsl2, Podman }

impl From<BackendArg> for SandboxKind {
    fn from(b: BackendArg) -> Self {
        match b {
            BackendArg::Native => SandboxKind::Native,
            BackendArg::Lima => SandboxKind::Lima,
            BackendArg::Wsl2 => SandboxKind::Wsl2,
            BackendArg::Podman => SandboxKind::Podman,
        }
    }
}

#[derive(Serialize, Debug)]
struct CompositeHealth {
    instance: String,
    native: clawops_core::native_ops::NativeDoctorReport,
    sandbox: clawops_core::sandbox_ops::SandboxDoctorReport,
    download: clawops_core::download_ops::DownloadDoctorReport,
    overall_healthy: bool,
}

fn parse_port_spec(s: &str) -> anyhow::Result<PortBinding> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    if parts.len() < 2 {
        anyhow::bail!("port spec must be host:guest[:label], got: {s}");
    }
    let host: u16 = parts[0].parse()
        .map_err(|e| anyhow::anyhow!("invalid host port `{}`: {e}", parts[0]))?;
    let guest: u16 = parts[1].parse()
        .map_err(|e| anyhow::anyhow!("invalid guest port `{}`: {e}", parts[1]))?;
    let label = parts.get(2).map(|s| s.to_string()).unwrap_or_default();
    Ok(PortBinding { host, guest, label })
}

pub async fn run(cmd: InstanceCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        InstanceCmd::List => {
            let o = InstanceOrchestrator::new();
            let list = o.list().await?;
            ctx.emit(&list)?;
        }
        InstanceCmd::Info { name } => {
            let o = InstanceOrchestrator::new();
            let i = o.info(&name).await?;
            ctx.emit(&i)?;
        }
        InstanceCmd::Create {
            name, claw, backend, sandbox_instance,
            ports, note, autoinstall_deps,
        } => {
            let parsed_ports: Vec<PortBinding> = ports.iter()
                .map(|s| parse_port_spec(s))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let o = InstanceOrchestrator::new();
            let report = o.create(CreateOpts {
                name, claw, backend: backend.into(),
                sandbox_instance, ports: parsed_ports, note,
                autoinstall_native_deps: autoinstall_deps,
            }, ProgressSink::noop()).await?;
            ctx.emit(&report)?;
        }
        InstanceCmd::Destroy { name } => {
            // Stream progress so the GUI's delete-progress dialog can
            // update its stage indicator (lookup → ports → destroy-vm
            // → remove from registry). Mirrors run_install's pattern.
            let (tx, mut rx) = tokio::sync::mpsc::channel::<clawops_core::ProgressEvent>(32);
            let sink = ProgressSink::new(tx);
            let printer = {
                let output = ctx.output.clone();
                let json = ctx.json;
                tokio::spawn(async move {
                    while let Some(ev) = rx.recv().await {
                        if json {
                            output.emit(crate::output::CliEvent::Progress {
                                stage: ev.stage.clone(),
                                percent: ev.percent.unwrap_or(0),
                                message: ev.message.clone(),
                            });
                        } else {
                            let pct = ev.percent.map(|p| format!("{p:>3}%"))
                                .unwrap_or_else(|| " … ".into());
                            println!("[{pct}] {:<14} {}", ev.stage, ev.message);
                        }
                    }
                })
            };
            let o = InstanceOrchestrator::new();
            let report = o.destroy(&name, sink).await?;
            let _ = printer.await;
            ctx.emit(&report)?;
        }
        InstanceCmd::Health => {
            let native_ops = DefaultNativeOps::new();
            let sandbox_ops: Box<dyn SandboxOps> = if cfg!(target_os = "macos") {
                Box::new(LimaOps::new(&ctx.instance))
            } else if cfg!(target_os = "windows") {
                Box::new(WslOps::new(&ctx.instance))
            } else {
                Box::new(PodmanOps::new(&ctx.instance))
            };
            let download_ops = CatalogBackedDownloadOps::with_defaults();

            let native = native_ops.doctor().await?;
            let sandbox = sandbox_ops.doctor().await?;
            let download = download_ops.doctor().await?;

            let overall_healthy = native.healthy() && sandbox.healthy() && download.healthy();
            let combined = CompositeHealth {
                instance: ctx.instance.clone(),
                native, sandbox, download, overall_healthy,
            };
            ctx.emit(&combined)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_port_spec_simple() {
        let p = parse_port_spec("3000:3000").unwrap();
        assert_eq!(p.host, 3000);
        assert_eq!(p.guest, 3000);
        assert!(p.label.is_empty());
    }

    #[test]
    fn parse_port_spec_with_label() {
        let p = parse_port_spec("3000:3001:gateway").unwrap();
        assert_eq!(p.host, 3000);
        assert_eq!(p.guest, 3001);
        assert_eq!(p.label, "gateway");
    }

    #[test]
    fn parse_port_spec_invalid_errs() {
        assert!(parse_port_spec("3000").is_err());
        assert!(parse_port_spec("abc:def").is_err());
    }
}
