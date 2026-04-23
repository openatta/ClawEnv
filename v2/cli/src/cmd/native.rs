use clap::Subcommand;
use clawops_core::native_ops::{DefaultNativeOps, NativeOps, VersionSpec};
use clawops_core::ProgressSink;

use crate::shared::Ctx;

#[derive(Subcommand)]
pub enum NativeCmd {
    /// Show ClawEnv home + node/git status.
    Status,
    /// List components with versions and disk usage.
    Components,
    /// Run diagnostics.
    Doctor,
    /// Repair specified issue IDs. (Stage A: returns Unsupported.)
    Repair {
        issue_ids: Vec<String>,
    },
    /// Upgrade node or git. (Stage A: returns Unsupported.)
    Upgrade {
        #[command(subcommand)]
        what: UpgradeWhat,
    },
}

#[derive(Subcommand)]
pub enum UpgradeWhat {
    Node { #[arg(long)] to: Option<String> },
    Git  { #[arg(long)] to: Option<String> },
}

pub async fn run(cmd: NativeCmd, ctx: &Ctx) -> anyhow::Result<()> {
    let ops = DefaultNativeOps::new();
    match cmd {
        NativeCmd::Status => {
            let s = ops.status().await?;
            ctx.emit(&s)?;
        }
        NativeCmd::Components => {
            let cs = ops.list_components().await?;
            ctx.emit(&cs)?;
        }
        NativeCmd::Doctor => {
            let r = ops.doctor().await?;
            ctx.emit(&r)?;
        }
        NativeCmd::Repair { issue_ids } => {
            ops.repair(&issue_ids, ProgressSink::noop()).await?;
            ctx.emit_text("repaired");
        }
        NativeCmd::Upgrade { what } => {
            let (name, target) = match what {
                UpgradeWhat::Node { to } => ("node", to),
                UpgradeWhat::Git { to } => ("git", to),
            };
            let spec = match target {
                Some(v) => VersionSpec::Exact(v),
                None => VersionSpec::Latest,
            };
            let res = match name {
                "node" => ops.upgrade_node(spec, ProgressSink::noop()).await,
                "git"  => ops.upgrade_git(spec, ProgressSink::noop()).await,
                _      => unreachable!(),
            };
            res?;
            ctx.emit_text(format!("upgraded {name}"));
        }
    }
    Ok(())
}
