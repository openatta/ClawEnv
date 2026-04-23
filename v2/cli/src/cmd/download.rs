use std::path::PathBuf;

use clap::Subcommand;
use clawops_core::download_ops::{CatalogBackedDownloadOps, DownloadOps};
use clawops_core::{CancellationToken, ProgressSink};

use crate::shared::Ctx;

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
                .filter(|a| os.as_deref().map_or(true, |o| a.platform.os == o))
                .filter(|a| arch.as_deref().map_or(true, |x| a.platform.arch == x))
                .collect();
            ctx.emit(&artifacts)?;
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
        DownloadCmd::Doctor => {
            let r = ops.doctor().await?;
            ctx.emit(&r)?;
        }
    }
    Ok(())
}
