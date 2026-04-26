use clap::Subcommand;
use clawops_core::native_ops::{DefaultNativeOps, NativeOps, VersionSpec};
use clawops_core::ProgressSink;

use crate::shared::{new_table, severity_color, Ctx};

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
            ctx.emit_pretty(&s, |st| {
                println!("ClawEnv home : {}", st.clawenv_home.display());
                println!("Home exists  : {}", if st.home_exists { "yes" } else { "no" });
                println!("Disk usage   : {}", format_bytes(st.total_disk_bytes));
                match &st.node {
                    Some(n) => println!(
                        "Node         : {} ({})  {}",
                        n.version,
                        n.path.display(),
                        if n.healthy { "✓" } else { "✗" }
                    ),
                    None => println!("Node         : not installed"),
                }
                match &st.git {
                    Some(g) => println!(
                        "Git          : {} ({})  {}",
                        g.version,
                        g.path.display(),
                        if g.healthy { "✓" } else { "✗" }
                    ),
                    None => println!("Git          : not installed"),
                }
            })?;
        }
        NativeCmd::Components => {
            let cs = ops.list_components().await?;
            ctx.emit_pretty(&cs, |rows| {
                let mut t = new_table(["name", "version", "path", "healthy", "size"]);
                for c in rows {
                    t.add_row([
                        c.name.clone(),
                        c.version.clone().unwrap_or_else(|| "—".into()),
                        c.path.as_ref().map_or("—".into(), |p| p.display().to_string()),
                        if c.healthy { "yes".into() } else { "no".into() },
                        format_bytes(c.size_bytes),
                    ]);
                }
                println!("{t}");
            })?;
        }
        NativeCmd::Doctor => {
            let r = ops.doctor().await?;
            ctx.emit_pretty(&r, |rep| {
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

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB { format!("{:.1} GB", n as f64 / GB as f64) }
    else if n >= MB { format!("{:.1} MB", n as f64 / MB as f64) }
    else if n >= KB { format!("{:.1} KB", n as f64 / KB as f64) }
    else { format!("{n} B") }
}
