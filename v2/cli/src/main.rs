//! clawcli — ClawEnv v2 unified CLI.
//!
//! Two-tier command surface:
//!
//! - **Verb layer** (task-oriented): `install`, `start`, `stop`, `status`,
//!   `logs`, `exec`, `shell`, `doctor`, … — what users want to DO.
//!   Each verb composes multiple Ops calls.
//! - **Noun layer** (layer-oriented): `claw`, `sandbox`, `native`,
//!   `download`, `proxy`, `instance`, … — direct access to one Ops
//!   layer, for scripts and power users.

mod cmd;
mod shared;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clawcli", version, about = "ClawEnv v2 unified CLI")]
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
    // ═══════════════════ Verb layer (task-oriented) ═══════════════════
    /// List all registered instances.
    List,
    /// Show aggregate status for an instance (VM state + claw + ports).
    Status {
        /// Instance name (defaults to --instance global or "default").
        name: Option<String>,
    },
    /// Start an instance's sandbox VM.
    Start { name: Option<String> },
    /// Stop an instance's sandbox VM.
    Stop { name: Option<String> },
    /// Restart an instance's sandbox VM.
    Restart { name: Option<String> },
    /// Run a command inside the sandbox non-interactively.
    /// Usage: clawcli exec <name> -- <cmd> [args...]
    Exec {
        name: Option<String>,
        /// Command and args, taken after `--`.
        #[arg(last = true)]
        cmd: Vec<String>,
    },
    /// Open an interactive shell inside the sandbox.
    Shell { name: Option<String> },
    /// Run aggregate diagnostics across native + sandbox + download.
    Doctor { name: Option<String> },
    /// Probe connectivity to the 3 load-bearing hosts (Alpine CDN,
    /// npm, github) from host or inside a sandbox.
    NetCheck {
        /// Probe from the host machine (`host`) or from inside a
        /// sandbox VM (`sandbox`, requires the instance to be up).
        #[arg(long, value_enum, default_value_t = cmd::verbs::NetCheckMode::Host)]
        mode: cmd::verbs::NetCheckMode,
        /// Instance name when --mode=sandbox. Falls back to --instance.
        name: Option<String>,
    },

    // ═══════════════════ Noun layer (direct ops access) ═══════════════
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
    /// Composed cross-layer instance operations (info / create / destroy).
    Instance {
        #[command(subcommand)]
        sub: cmd::instance::InstanceCmd,
    },
    /// Proxy and credential management.
    Proxy {
        #[command(subcommand)]
        sub: cmd::proxy::ProxyCmd,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,clawops_cli=info,clawops_core=info".into())
        )
        .init();

    let cli = Cli::parse();
    let ctx = shared::Ctx {
        json: cli.json,
        quiet: cli.quiet,
        instance: cli.instance.clone().unwrap_or_else(|| "default".into()),
    };

    match cli.command {
        // Verb layer
        Command::List               => cmd::verbs::run_list(&ctx).await,
        Command::Status { name }    => cmd::verbs::run_status(&ctx, name).await,
        Command::Start  { name }    => cmd::verbs::run_start(&ctx, name).await,
        Command::Stop   { name }    => cmd::verbs::run_stop(&ctx, name).await,
        Command::Restart{ name }    => cmd::verbs::run_restart(&ctx, name).await,
        Command::Exec   { name, cmd } => cmd::verbs::run_exec(&ctx, name, cmd).await,
        Command::Shell  { name }    => cmd::verbs::run_shell(&ctx, name).await,
        Command::Doctor { name }    => cmd::verbs::run_doctor(&ctx, name).await,
        Command::NetCheck { mode, name } => cmd::verbs::run_net_check(&ctx, mode, name).await,
        // Noun layer
        Command::Claw { sub }       => cmd::claw::run(sub, &ctx).await,
        Command::Sandbox { sub }    => cmd::sandbox::run(sub, &ctx).await,
        Command::Native { sub }     => cmd::native::run(sub, &ctx).await,
        Command::Download { sub }   => cmd::download::run(sub, &ctx).await,
        Command::Instance { sub }   => cmd::instance::run(sub, &ctx).await,
        Command::Proxy { sub }      => cmd::proxy::run(sub, &ctx).await,
    }
}
