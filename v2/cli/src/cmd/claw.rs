use std::sync::Arc;

use clap::Subcommand;
use clawops_core::claw_ops::{ClawRegistry, DoctorOpts, LogsOpts, UpdateOpts};
use clawops_core::runners::{LocalProcessRunner, SandboxCommandRunner};
use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};
use clawops_core::{CancellationToken, CommandRunner, CommandSpec};
use serde::Serialize;

use crate::shared::{new_table, Ctx};

#[derive(Subcommand)]
pub enum ClawCmd {
    /// List known Claw products.
    List,
    /// Preview (default) or execute (`--execute`) a claw `update`.
    Update {
        claw: String,
        #[arg(long)] yes: bool,
        #[arg(long)] json: bool,
        #[arg(long)] dry_run: bool,
        #[arg(long)] channel: Option<String>,
        #[arg(long)] tag: Option<String>,
        #[arg(long)] no_restart: bool,
        #[arg(long)] execute: bool,
        #[arg(long, value_enum)] backend: Option<super::sandbox::BackendSel>,
    },
    Doctor {
        claw: String,
        #[arg(long)] fix: bool,
        #[arg(long)] json: bool,
        #[arg(long)] execute: bool,
        #[arg(long, value_enum)] backend: Option<super::sandbox::BackendSel>,
    },
    Status {
        claw: String,
        #[arg(long)] execute: bool,
        #[arg(long, value_enum)] backend: Option<super::sandbox::BackendSel>,
    },
    Version {
        claw: String,
        #[arg(long)] execute: bool,
        #[arg(long, value_enum)] backend: Option<super::sandbox::BackendSel>,
    },
    Logs {
        claw: String,
        #[arg(long)] tail: Option<u32>,
        #[arg(long)] follow: bool,
        #[arg(long)] level: Option<String>,
        #[arg(long)] execute: bool,
        #[arg(long, value_enum)] backend: Option<super::sandbox::BackendSel>,
    },
    Config {
        claw: String,
        #[command(subcommand)] op: ConfigOp,
        #[arg(long)] execute: bool,
        #[arg(long, value_enum)] backend: Option<super::sandbox::BackendSel>,
    },
}

#[derive(Subcommand)]
pub enum ConfigOp {
    Get { key: String },
    Set { key: String, value: String },
    List,
}

#[derive(Serialize, Debug)]
struct ClawEntry {
    id: String,
    binary: String,
    supports_native: bool,
}

#[derive(Serialize, Debug)]
struct CommandPreview {
    claw: String,
    binary: String,
    args: Vec<String>,
    timeout_secs: Option<u64>,
    output_format: String,
}

#[derive(Serialize, Debug)]
struct ExecutionReport {
    claw: String,
    runner: String,
    exit_code: i32,
    duration_ms: u64,
    was_cancelled: bool,
    was_timed_out: bool,
    stdout: String,
    stderr: String,
    structured: Option<serde_json::Value>,
}

fn preview(claw_id: &str, spec: &CommandSpec) -> CommandPreview {
    CommandPreview {
        claw: claw_id.into(),
        binary: spec.binary.clone(),
        args: spec.args.clone(),
        timeout_secs: spec.timeout.map(|d| d.as_secs()),
        output_format: format!("{:?}", spec.output_format),
    }
}

/// Resolve which runner to use for --execute. If `backend` is None we
/// default to host-native (LocalProcessRunner). If a sandbox backend is
/// specified, we wrap v2's SandboxBackend impl in a SandboxCommandRunner.
fn runner_for(
    backend: Option<super::sandbox::BackendSel>,
    instance: &str,
) -> Box<dyn CommandRunner> {
    match backend {
        None => Box::new(LocalProcessRunner::new()),
        Some(super::sandbox::BackendSel::Lima) => {
            let b: Arc<dyn SandboxBackend> = Arc::new(LimaBackend::new(instance));
            Box::new(SandboxCommandRunner::new(b))
        }
        Some(super::sandbox::BackendSel::Wsl2) => {
            let b: Arc<dyn SandboxBackend> = Arc::new(WslBackend::new(instance));
            Box::new(SandboxCommandRunner::new(b))
        }
        Some(super::sandbox::BackendSel::Podman) => {
            let b: Arc<dyn SandboxBackend> = Arc::new(PodmanBackend::new(instance));
            Box::new(SandboxCommandRunner::new(b))
        }
    }
}

async fn run_spec_and_emit(
    ctx: &Ctx,
    claw_id: &str,
    spec: CommandSpec,
    backend: Option<super::sandbox::BackendSel>,
) -> anyhow::Result<()> {
    let runner = runner_for(backend, &ctx.instance);
    let runner_name = runner.name().to_string();
    let report = match runner.exec(spec, CancellationToken::new()).await {
        Ok(res) => ExecutionReport {
            claw: claw_id.into(),
            runner: runner_name,
            exit_code: res.exit_code,
            duration_ms: res.duration.as_millis() as u64,
            was_cancelled: res.was_cancelled,
            was_timed_out: res.was_timed_out,
            stdout: res.stdout,
            stderr: res.stderr,
            structured: res.structured,
        },
        Err(e) => {
            // Emit structured error so --json consumers still get a
            // parseable payload (exit_code -2 signals runner failure;
            // distinguishable from any legitimate process exit code).
            ExecutionReport {
                claw: claw_id.into(),
                runner: runner_name,
                exit_code: -2,
                duration_ms: 0,
                was_cancelled: false,
                was_timed_out: false,
                stdout: String::new(),
                stderr: format!("runner error: {e}"),
                structured: None,
            }
        }
    };
    let failed = report.exit_code != 0 && !report.was_cancelled && !report.was_timed_out;
    ctx.emit(&report)?;
    if failed {
        // Use exit(1) rather than passing through the child code — negative
        // codes don't map to shell exit statuses cleanly.
        std::process::exit(1);
    }
    Ok(())
}

async fn preview_or_execute(
    ctx: &Ctx,
    claw_id: &str,
    spec: CommandSpec,
    execute: bool,
    backend: Option<super::sandbox::BackendSel>,
) -> anyhow::Result<()> {
    if execute {
        run_spec_and_emit(ctx, claw_id, spec, backend).await
    } else {
        ctx.emit(&preview(claw_id, &spec))
    }
}

pub async fn run(cmd: ClawCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        ClawCmd::List => {
            let entries: Vec<ClawEntry> = ClawRegistry::all().into_iter().map(|c| ClawEntry {
                id: c.id().into(),
                binary: c.binary().into(),
                supports_native: c.supports_native(),
            }).collect();
            ctx.emit_pretty(&entries, |rows| {
                let mut t = new_table(["id", "binary", "native"]);
                for r in rows {
                    t.add_row([
                        r.id.clone(),
                        r.binary.clone(),
                        if r.supports_native { "yes" } else { "no" }.to_string(),
                    ]);
                }
                println!("{t}");
            })?;
        }
        ClawCmd::Update { claw, yes, json, dry_run, channel, tag, no_restart, execute, backend } => {
            let cli = ClawRegistry::cli_for(&claw)
                .ok_or_else(|| anyhow::anyhow!("unknown claw: {claw}"))?;
            let spec = cli.update(UpdateOpts {
                non_interactive: yes, json, dry_run, channel, tag, no_restart,
            });
            preview_or_execute(ctx, &claw, spec, execute, backend).await?;
        }
        ClawCmd::Doctor { claw, fix, json, execute, backend } => {
            let cli = ClawRegistry::cli_for(&claw)
                .ok_or_else(|| anyhow::anyhow!("unknown claw: {claw}"))?;
            let spec = cli.doctor(DoctorOpts { fix, json });
            preview_or_execute(ctx, &claw, spec, execute, backend).await?;
        }
        ClawCmd::Status { claw, execute, backend } => {
            let cli = ClawRegistry::cli_for(&claw)
                .ok_or_else(|| anyhow::anyhow!("unknown claw: {claw}"))?;
            preview_or_execute(ctx, &claw, cli.status(), execute, backend).await?;
        }
        ClawCmd::Version { claw, execute, backend } => {
            let cli = ClawRegistry::cli_for(&claw)
                .ok_or_else(|| anyhow::anyhow!("unknown claw: {claw}"))?;
            preview_or_execute(ctx, &claw, cli.version(), execute, backend).await?;
        }
        ClawCmd::Logs { claw, tail, follow, level, execute, backend } => {
            let cli = ClawRegistry::cli_for(&claw)
                .ok_or_else(|| anyhow::anyhow!("unknown claw: {claw}"))?;
            let spec = cli.logs(LogsOpts { tail, follow, level });
            preview_or_execute(ctx, &claw, spec, execute, backend).await?;
        }
        ClawCmd::Config { claw, op, execute, backend } => {
            let cli = ClawRegistry::cli_for(&claw)
                .ok_or_else(|| anyhow::anyhow!("unknown claw: {claw}"))?;
            let spec = match op {
                ConfigOp::Get { key } => cli.config_get(&key),
                ConfigOp::Set { key, value } => cli.config_set(&key, &value),
                ConfigOp::List => cli.config_list(),
            };
            preview_or_execute(ctx, &claw, spec, execute, backend).await?;
        }
    }
    Ok(())
}
