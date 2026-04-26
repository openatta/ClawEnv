//! clawcli — ClawEnv v2 unified CLI.
//!
//! Verb taxonomy per `v2/docs/CLI-DESIGN.md` §2:
//!
//! - **Lifecycle** (top-level, task-oriented): `install`, `uninstall`,
//!   `upgrade`, `start`, `stop`, `restart`, `launch`.
//! - **Inspection**: `list`, `status`, `info`, `logs`, `exec`, `shell`,
//!   `doctor`, `net-check`, `token`.
//! - **Distribution**: `export`, `import`.
//! - **Configuration**: `config` group (show/get/set/unset/validate).
//! - **Self-introspection**: `system` group (info/version/state).
//! - **Layer-direct nouns**: `claw`, `sandbox`, `native`, `download`,
//!   `instance`, `proxy`, `bridge`, `browser`.
//!
//! Global flags: `--json`, `--quiet`, `--instance`, `--no-progress`.
//! See CLI-DESIGN.md §3-4 for argv conventions and JSON event protocol.

mod cmd;
mod output;
mod shared;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clawcli", version, about = "ClawEnv v2 unified CLI")]
pub struct Cli {
    /// Output results as line-delimited JSON CliEvent stream.
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-error output.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Suppress progress events (Data + Complete still emitted).
    #[arg(long, global = true)]
    pub no_progress: bool,

    /// Fallback target instance for verbs that take `<name>` positionally.
    #[arg(long, global = true)]
    pub instance: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    // ═══════════════════════ Lifecycle ═══════════════════════════════════
    /// End-to-end install: create instance + provision backend + deploy
    /// claw + register. Streams progress events; emits `InstallReport`.
    Install(cmd::verbs::InstallArgs),
    /// End-to-end teardown: stop daemons + destroy backend + remove
    /// from registry. Streams progress events.
    Uninstall(cmd::verbs::UninstallArgs),
    /// Upgrade an instance's claw to a new version (reuses VM).
    /// `--check` returns `UpdateCheckResponse` without installing.
    Upgrade(cmd::verbs::UpgradeArgs),
    /// Bring an instance's backend online (start VM; no daemon spawn).
    Start { name: Option<String> },
    /// Take an instance's backend offline. `--all` stops every
    /// registered instance (best-effort; per-instance failures logged).
    Stop {
        name: Option<String>,
        #[arg(long)] all: bool,
        /// Hard limit on the backend stop call.
        #[arg(long, default_value_t = 60)] timeout_secs: u64,
    },
    /// Restart an instance's backend.
    Restart { name: Option<String> },
    /// Spawn the gateway/dashboard daemons inside the instance and
    /// probe ready_port. Emits `LaunchReport`.
    Launch(cmd::verbs::LaunchArgs),

    // ═══════════════════════ Inspection ══════════════════════════════════
    /// List registered instances. `--filter` narrows by backend or
    /// health. Emits `ListResponse`.
    List {
        #[arg(long)] filter: Option<String>,
        #[arg(long)] include_broken: bool,
    },
    /// Show one instance: VM state + claw + ports + caps. Emits
    /// `StatusResponse`. `--no-probe` skips the backend availability
    /// check (registry-only, fast).
    Status {
        name: Option<String>,
        #[arg(long)] no_probe: bool,
    },
    /// Registry-only instance record (fast, never probes the backend).
    /// Emits `InstanceConfig`.
    Info { name: String },
    /// Tail or follow gateway/dashboard logs. Streams data events when
    /// `--follow`; otherwise emits one `LogResponse`.
    Logs {
        name: String,
        #[arg(long)] follow: bool,
        #[arg(long, default_value_t = 200)] tail: u32,
        /// e.g. `5m`, `1h` (max age of the oldest line returned).
        #[arg(long)] since: Option<String>,
    },
    /// Run a command non-interactively inside an instance.
    /// Usage: `clawcli exec <name> -- <cmd> [args...]`. Emits `ExecResult`.
    /// (For an attached TTY use `clawcli shell <name>`.)
    Exec {
        name: Option<String>,
        #[arg(last = true)] cmd: Vec<String>,
    },
    /// Open an interactive shell inside an instance (passthrough; no
    /// JSON events even with --json).
    Shell { name: Option<String> },
    /// Aggregate diagnostics. With `<name>` scopes to one instance;
    /// with `--all` iterates every registered instance. `--fix`
    /// applies repair recipes for fixable issues.
    Doctor {
        name: Option<String>,
        #[arg(long)] all: bool,
        #[arg(long)] fix: bool,
    },
    /// Connectivity probes. `--mode host` (default) probes from the
    /// host process; `--mode sandbox <name>` probes from inside an
    /// instance VM. Emits `NetCheckReport`.
    NetCheck {
        #[arg(long, value_enum, default_value_t = cmd::verbs::NetCheckMode::Host)]
        mode: cmd::verbs::NetCheckMode,
        name: Option<String>,
        #[arg(long)] proxy_url: Option<String>,
    },
    /// Read the gateway auth token from inside an instance. Plain
    /// stdout in non-JSON mode; `{token: string}` Data event in JSON.
    Token { name: Option<String> },

    // ═══════════════════════ Distribution ════════════════════════════════
    /// Export an instance to a portable bundle (tar.gz with manifest +
    /// payload). Emits `ExportReport`.
    Export(cmd::verbs::ExportArgs),
    /// Import a bundle as a new instance. Validates the manifest,
    /// restores the backend image, registers it. Emits `ImportReport`.
    Import(cmd::verbs::ImportArgs),

    // ═══════════════════════ Configuration ═══════════════════════════════
    /// Read or write the global `~/.clawenv/config.toml`.
    Config {
        #[command(subcommand)] sub: cmd::config::ConfigCmd,
    },

    // ═══════════════════════ Self-introspection ══════════════════════════
    /// Host environment info, clawcli version, GUI launcher state.
    System {
        #[command(subcommand)] sub: cmd::system::SystemCmd,
    },

    // ═══════════════════════ Layer-direct nouns ══════════════════════════
    /// Invoke a claw product's own CLI (passthrough).
    Claw {
        #[command(subcommand)] sub: cmd::claw::ClawCmd,
    },
    /// Sandbox VM ops not part of the lifecycle: edit/rename/port/prereqs/stats.
    Sandbox {
        #[command(subcommand)] sub: cmd::sandbox::SandboxCmd,
    },
    /// Host-runtime ops: components, upgrade node/git, repair.
    Native {
        #[command(subcommand)] sub: cmd::native::NativeCmd,
    },
    /// Artifact catalog: list, fetch, doctor.
    Download {
        #[command(subcommand)] sub: cmd::download::DownloadCmd,
    },
    /// Registry-only operations (info/create/destroy/health).
    Instance {
        #[command(subcommand)] sub: cmd::instance::InstanceCmd,
    },
    /// Proxy config + apply (resolve/get/set/check/apply/clear/set-password).
    Proxy {
        #[command(subcommand)] sub: cmd::proxy::ProxyCmd,
    },
    /// AttaRun bridge daemon (config/start/stop/status).
    Bridge {
        #[command(subcommand)] sub: cmd::bridge::BridgeCmd,
    },
    /// Chromium HIL state machine inside an instance (status/hil-start/hil-resume).
    Browser {
        #[command(subcommand)] sub: cmd::browser::BrowserCmd,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,clawops_cli=info,clawops_core=info".into())
        )
        .init();

    let cli = Cli::parse();
    let output = output::Output::new(cli.json);
    let ctx = shared::Ctx {
        json: cli.json,
        quiet: cli.quiet,
        no_progress: cli.no_progress,
        instance: cli.instance.clone().unwrap_or_else(|| "default".into()),
        output: output.clone(),
    };

    let result = run_command(cli.command, &ctx).await;

    if cli.json {
        match &result {
            Ok(()) => output.emit(output::CliEvent::Complete {
                message: "ok".into(),
            }),
            Err(e) => output.emit(output::CliEvent::Error {
                message: format!("{e:#}"),
                code: None,
            }),
        }
    }
    result
}

async fn run_command(command: Command, ctx: &shared::Ctx) -> anyhow::Result<()> {
    match command {
        // Lifecycle
        Command::Install(args)      => cmd::verbs::run_install(ctx, args).await,
        Command::Uninstall(args)    => cmd::verbs::run_uninstall(ctx, args).await,
        Command::Upgrade(args)      => cmd::verbs::run_upgrade(ctx, args).await,
        Command::Start  { name }    => cmd::verbs::run_start(ctx, name).await,
        Command::Stop   { name, all, timeout_secs }
            => cmd::verbs::run_stop(ctx, name, all, timeout_secs).await,
        Command::Restart{ name }    => cmd::verbs::run_restart(ctx, name).await,
        Command::Launch(args)       => cmd::verbs::run_launch(ctx, args).await,

        // Inspection
        Command::List { filter, include_broken }
            => cmd::verbs::run_list(ctx, filter, include_broken).await,
        Command::Status { name, no_probe }
            => cmd::verbs::run_status(ctx, name, no_probe).await,
        Command::Info  { name }     => cmd::verbs::run_info(ctx, name).await,
        Command::Logs  { name, follow, tail, since }
            => cmd::verbs::run_logs(ctx, name, follow, tail, since).await,
        Command::Exec  { name, cmd } => cmd::verbs::run_exec(ctx, name, cmd).await,
        Command::Shell { name }     => cmd::verbs::run_shell(ctx, name).await,
        Command::Doctor{ name, all, fix }
            => cmd::verbs::run_doctor(ctx, name, all, fix).await,
        Command::NetCheck { mode, name, proxy_url }
            => cmd::verbs::run_net_check(ctx, mode, name, proxy_url).await,
        Command::Token { name }     => cmd::verbs::run_token(ctx, name).await,

        // Distribution
        Command::Export(args)       => cmd::verbs::run_export(ctx, args).await,
        Command::Import(args)       => cmd::verbs::run_import(ctx, args).await,

        // Configuration & self-introspection
        Command::Config { sub }     => cmd::config::run(sub, ctx).await,
        Command::System { sub }     => cmd::system::run(sub, ctx).await,

        // Layer-direct
        Command::Claw    { sub }    => cmd::claw::run(sub, ctx).await,
        Command::Sandbox { sub }    => cmd::sandbox::run(sub, ctx).await,
        Command::Native  { sub }    => cmd::native::run(sub, ctx).await,
        Command::Download{ sub }    => cmd::download::run(sub, ctx).await,
        Command::Instance{ sub }    => cmd::instance::run(sub, ctx).await,
        Command::Proxy   { sub }    => cmd::proxy::run(sub, ctx).await,
        Command::Bridge  { sub }    => cmd::bridge::run(sub, ctx).await,
        Command::Browser { sub }    => cmd::browser::run(sub, ctx).await,
    }
}
