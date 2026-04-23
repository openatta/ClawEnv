//! clawops — ClawEnv v2 unified CLI.
//!
//! Five command groups reflecting the layered architecture in
//! `v2/docs/DESIGN.md`:
//!   - claw       (ClawOps)
//!   - sandbox    (SandboxOps)
//!   - native     (NativeOps)
//!   - download   (DownloadOps)
//!   - instance   (composed)

mod cmd;
mod shared;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clawops", version, about = "ClawEnv v2 unified CLI")]
pub struct Cli {
    /// Output results as JSON (machine-readable).
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-error output.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Target instance name (falls back to "default").
    #[arg(long, global = true)]
    pub instance: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage Claw products (Hermes, OpenClaw) via their own CLI.
    Claw {
        #[command(subcommand)]
        sub: cmd::claw::ClawCmd,
    },
    /// Manage the sandbox VM (Lima/WSL2/Podman).
    Sandbox {
        #[command(subcommand)]
        sub: cmd::sandbox::SandboxCmd,
    },
    /// Manage host-side runtime (node, git, ~/.clawenv).
    Native {
        #[command(subcommand)]
        sub: cmd::native::NativeCmd,
    },
    /// Manage downloadable artifacts (Node.js, Git, VM images, ...).
    Download {
        #[command(subcommand)]
        sub: cmd::download::DownloadCmd,
    },
    /// Composed cross-layer instance operations.
    Instance {
        #[command(subcommand)]
        sub: cmd::instance::InstanceCmd,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,clawops=info".into())
        )
        .init();

    let cli = Cli::parse();
    let ctx = shared::Ctx {
        json: cli.json,
        quiet: cli.quiet,
        instance: cli.instance.clone().unwrap_or_else(|| "default".into()),
    };

    match cli.command {
        Command::Claw { sub }       => cmd::claw::run(sub, &ctx).await,
        Command::Sandbox { sub }    => cmd::sandbox::run(sub, &ctx).await,
        Command::Native { sub }     => cmd::native::run(sub, &ctx).await,
        Command::Download { sub }   => cmd::download::run(sub, &ctx).await,
        Command::Instance { sub }   => cmd::instance::run(sub, &ctx).await,
    }
}
