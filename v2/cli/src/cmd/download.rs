use std::path::PathBuf;

use clap::Subcommand;
use clawops_core::download_ops::{CatalogBackedDownloadOps, DownloadOps};
use clawops_core::{CancellationToken, ProgressSink};

use crate::shared::{new_table, Ctx};

#[derive(Subcommand)]
pub enum DownloadCmd {
    /// List catalog entries, optionally filtered by platform.
    List {
        #[arg(long)] os: Option<String>,
        #[arg(long)] arch: Option<String>,
    },
    /// Cache management.
    Cache {
        #[command(subcommand)]
        op: CacheOp,
    },
    /// Fetch an artifact into the cache (or a custom path with --to).
    Fetch {
        name: String,
        #[arg(long)] version: Option<String>,
        #[arg(long)] to: Option<PathBuf>,
    },
    /// Check connectivity to each host in the catalog.
    CheckConnectivity,
    /// Fast go/no-go probe against the 3 load-bearing hosts (Alpine CDN,
    /// npm, GitHub). Much cheaper than check-connectivity; use this
    /// before kicking off an install.
    Preflight,
    /// Run diagnostics.
    Doctor,
}

#[derive(Subcommand)]
pub enum CacheOp {
    /// List currently cached items.
    List,
    /// Verify cached items against catalog sha256.
    Verify,
    /// Prune old versions, keeping N most recent per artifact.
    Prune {
        #[arg(long, default_value_t = 2)]
        keep: usize,
    },
}

pub async fn run(cmd: DownloadCmd, ctx: &Ctx) -> anyhow::Result<()> {
    let ops = CatalogBackedDownloadOps::with_defaults();
    match cmd {
        DownloadCmd::List { os, arch } => {
            let artifacts: Vec<_> = ops.catalog().artifacts().iter()
                .filter(|a| os.as_deref().is_none_or(|o| a.platform.os == o))
                .filter(|a| arch.as_deref().is_none_or(|x| a.platform.arch == x))
                .cloned()
                .collect();
            ctx.emit_pretty(&artifacts, |rows| {
                let mut t = new_table(["name", "version", "os", "arch", "kind", "size"]);
                for a in rows {
                    t.add_row([
                        a.name.clone(),
                        a.version.clone(),
                        a.platform.os.clone(),
                        a.platform.arch.clone(),
                        format!("{:?}", a.kind).to_lowercase(),
                        a.size_hint.map_or("—".into(), format_bytes),
                    ]);
                }
                println!("{t}");
            })?;
        }
        DownloadCmd::Cache { op } => match op {
            CacheOp::List => {
                let items = ops.list_cached().await?;
                ctx.emit(&items)?;
            }
            CacheOp::Verify => {
                let items = ops.list_cached().await?;
                let mut results = Vec::new();
                for it in items {
                    let ok = ops.verify_cached(&it).await.unwrap_or(false);
                    results.push(serde_json::json!({
                        "name": it.name,
                        "version": it.version,
                        "verified": ok,
                    }));
                }
                ctx.emit(&results)?;
            }
            CacheOp::Prune { keep } => {
                let r = ops.prune_cache(keep).await?;
                ctx.emit(&r)?;
            }
        },
        DownloadCmd::Fetch { name, version, to } => {
            let cancel = CancellationToken::new();
            let sink = ProgressSink::noop();
            let path = match to {
                Some(dest) => {
                    ops.fetch_to(&name, version.as_deref(), &dest, sink, cancel).await?
                        .path
                }
                None => ops.fetch(&name, version.as_deref(), sink, cancel).await?,
            };
            ctx.emit(&serde_json::json!({ "path": path }))?;
        }
        DownloadCmd::CheckConnectivity => {
            let r = ops.check_connectivity().await?;
            ctx.emit(&r)?;
        }
        DownloadCmd::Preflight => {
            let r = clawops_core::preflight::run_preflight().await?;
            ctx.emit_pretty(&r, |rep| {
                let mut t = new_table(["host", "reachable", "status", "latency", "error"]);
                for h in &rep.hosts {
                    t.add_row([
                        h.host.clone(),
                        if h.reachable { "yes".into() } else { "no".into() },
                        h.http_status.map_or("—".into(), |s| s.to_string()),
                        h.latency_ms.map_or("—".into(), |ms| format!("{ms} ms")),
                        h.error.clone().unwrap_or_else(|| "—".into()),
                    ]);
                }
                println!("{t}");
                if let Some(p) = &rep.http_proxy_env {
                    println!("HTTP_PROXY  = {p}");
                }
                if let Some(p) = &rep.https_proxy_env {
                    println!("HTTPS_PROXY = {p}");
                }
                if let Some(s) = &rep.suggestion {
                    println!("\n{s}");
                }
            })?;
            if !r.all_reachable {
                // Exit nonzero so scripts and install orchestration can gate on it.
                std::process::exit(1);
            }
        }
        DownloadCmd::Doctor => {
            let r = ops.doctor().await?;
            ctx.emit(&r)?;
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
