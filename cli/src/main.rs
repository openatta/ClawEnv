use anyhow::Result;
use clap::{Parser, Subcommand};

mod output;
use output::{Output, CliEvent};

#[derive(Parser)]
#[command(name = "clawenv", version, about = "Claw ecosystem sandbox installer & manager")]
struct Cli {
    /// Output format: human-readable (default) or JSON lines for GUI integration
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install a claw product (sandbox or native mode)
    Install {
        #[arg(long, default_value = "sandbox")]
        mode: String,
        #[arg(long, default_value = "openclaw")]
        claw_type: String,
        #[arg(long, default_value = "latest")]
        version: String,
        #[arg(long, default_value = "default")]
        name: String,
        #[arg(long)]
        image: Option<String>,
        #[arg(long)]
        browser: bool,
        #[arg(long, default_value = "0")]
        port: u16,
        /// API key for the claw product
        #[arg(long)]
        api_key: Option<String>,
        /// Developer mode: run a single install step instead of full install.
        /// Steps: prereq, create, claw, config, gateway.
        /// Omit for full install (normal user flow).
        #[arg(long)]
        step: Option<String>,
    },
    /// Uninstall an instance
    Uninstall {
        #[arg(long, default_value = "default")]
        name: String,
    },
    /// List all instances
    List,
    /// Start an instance
    Start { name: Option<String> },
    /// Stop an instance
    Stop { name: Option<String> },
    /// Restart an instance
    Restart { name: Option<String> },
    /// Show instance status
    Status { name: Option<String> },
    /// Show instance logs
    Logs {
        name: Option<String>,
        #[arg(short, long)]
        follow: bool,
    },
    /// Upgrade to latest or specific version
    Upgrade {
        name: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    /// Check for available updates
    UpdateCheck { name: Option<String> },
    /// Export instance as distributable package
    Export {
        name: Option<String>,
        #[arg(long, default_value = "./packages")]
        output: String,
    },
    /// Import instance from a package file
    Import {
        file: String,
        #[arg(long, default_value = "default")]
        name: String,
    },
    /// Diagnose current environment
    Doctor,
    /// Execute a command inside the sandbox
    Exec {
        cmd: String,
        name: Option<String>,
    },
    /// List available claw types from registry
    ClawTypes,
    /// System check (OS, memory, disk, sandbox backend)
    SystemCheck,
    /// Rename an instance
    Rename {
        old_name: String,
        new_name: String,
    },
    /// Edit instance resources or ports
    Edit {
        name: String,
        #[arg(long)]
        cpus: Option<u32>,
        #[arg(long)]
        memory: Option<u32>,
        #[arg(long)]
        disk: Option<u32>,
        #[arg(long)]
        gateway_port: Option<u16>,
        #[arg(long)]
        ttyd_port: Option<u16>,
    },
    /// Sandbox VM management
    #[command(subcommand)]
    Sandbox(SandboxCmd),
    /// Configuration management
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Bridge server management
    #[command(subcommand)]
    Bridge(BridgeCmd),
}

#[derive(Subcommand)]
enum BridgeCmd {
    /// Test bridge server connectivity
    Test {
        /// Bridge port (default: from config or 3100)
        #[arg(long)]
        port: Option<u16>,
    },
}

#[derive(Subcommand)]
enum SandboxCmd {
    /// List all VMs/containers on the system
    List,
    /// Show sandbox disk usage
    Info,
    /// Open interactive shell in sandbox
    Shell { name: Option<String> },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Show current configuration
    Show,
    /// Set a configuration value (bridge.permissions requires editing ~/.clawenv/config.toml directly)
    Set {
        key: String,
        value: String,
    },
    /// Test proxy connectivity
    ProxyTest,
}

fn resolve_name(name: Option<String>) -> String {
    name.unwrap_or_else(|| "default".into())
}

/// Validate an export `--output` value and produce its absolute/canonicalish
/// path. Export used to be repetitive across four backend arms; extracting
/// this kills ~25 lines of dupe. Two failure modes worth surfacing loudly:
///
///   1. User pointed at an existing directory. `PathBuf::is_dir()` matches
///      anything that *currently exists* as a directory — in past versions
///      the code silently treated this as "append a default filename" and
///      nested the tarball under a left-over directory from a previous
///      failed run.
///   2. Parent dir doesn't exist. Create it (best-effort) rather than
///      failing with an opaque tar errno.
fn validate_export_out_path(output: &str) -> Result<std::path::PathBuf> {
    let out_path = std::path::PathBuf::from(output);
    if out_path.is_dir() {
        anyhow::bail!(
            "Refusing to export: --output '{}' is a directory. \
             Pass a full .tar.gz filename.",
            out_path.display()
        );
    }
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(out_path)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let out = Output::new(cli.json);

    // Only init tracing for human mode (JSON mode should be clean stdout)
    if !cli.json {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .init();
    }

    // Inject proxy env vars from config (if configured) so all child
    // processes (npm, curl, etc.) inherit proxy settings automatically.
    if let Ok(config) = clawenv_core::config::ConfigManager::load() {
        clawenv_core::config::proxy::inject_proxy_env(&config.config().clawenv.proxy);
    }

    // Pin LIMA_HOME to ~/.clawenv/lima so any limactl invocation uses the
    // private data directory instead of the system default ~/.lima.
    #[cfg(target_os = "macos")]
    clawenv_core::sandbox::init_lima_env();

    // Pin Podman's XDG_DATA_HOME / XDG_RUNTIME_DIR to ~/.clawenv/podman-*
    // so all containers/images/volumes/db live inside our private tree,
    // matching Lima and WSL. CLI and GUI both must init so either entry
    // point sees the same storage.
    #[cfg(target_os = "linux")]
    clawenv_core::sandbox::init_podman_env();

    // Parent-death watchdog. macOS lacks Linux's PR_SET_PDEATHSIG, so the
    // clawgui sidecar (this process) can outlive a force-quit parent and
    // leave orphan limactl/hostagent/VM processes running. Poll getppid()
    // every second; when the parent becomes init (PID 1), SIGTERM our whole
    // process group so limactl children wind down with us.
    #[cfg(unix)]
    {
        let initial_ppid = unsafe { libc::getppid() };
        if initial_ppid > 1 {
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    let ppid = unsafe { libc::getppid() };
                    if ppid == 1 {
                        eprintln!("clawcli: parent ({initial_ppid}) died — terminating process group");
                        // Send SIGTERM to our entire process group. limactl
                        // handles this gracefully (stops VM + hostagent) and
                        // we'll receive it ourselves to exit cleanly.
                        unsafe { libc::killpg(libc::getpgrp(), libc::SIGTERM); }
                        // Hard kill after a grace window in case anyone
                        // ignored SIGTERM.
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                        unsafe { libc::killpg(libc::getpgrp(), libc::SIGKILL); }
                        std::process::exit(130);
                    }
                }
            });
        }
    }

    let result = run(cli.command, &out).await;

    match result {
        Ok(()) => {
            std::process::exit(0);
        }
        Err(e) => {
            // Classify error for structured frontend handling
            let code = classify_error(&e);
            out.emit(CliEvent::Error { message: e.to_string(), code: Some(code) });
            std::process::exit(1);
        }
    }
}

async fn run(command: Commands, out: &Output) -> Result<()> {
    use clawenv_core::api::*;
    use clawenv_core::config::{ConfigManager, UserMode};

    // One-shot legacy migration: patches pre-v0.2.7 instances that are
    // missing dashboard_port. Idempotent and best-effort — a failure here
    // (e.g. read-only config) shouldn't prevent the command from running
    // on instances that don't need migration. Runs before `match command`
    // so every subcommand sees the migrated config view.
    if let Ok(mut cfg) = ConfigManager::load() {
        let registry = clawenv_core::claw::ClawRegistry::load();
        if let Err(e) = clawenv_core::manager::instance::migrate_instance_ports(&mut cfg, &registry) {
            tracing::warn!("dashboard_port migration skipped: {e}");
        }
    }

    match command {
        // ====== Install ======
        Commands::Install { mode, claw_type, version, name, image, browser, port, api_key, step } => {
            use clawenv_core::manager::install::{self, InstallOptions};
            use clawenv_core::sandbox::{InstallMode, ImageSource};

            let claw_reg = clawenv_core::claw::ClawRegistry::load();
            let desc = claw_reg.get_strict(&claw_type)?;

            let install_mode = if let Some(ref img_path) = image {
                InstallMode::PrebuiltImage {
                    source: ImageSource::LocalFile { path: img_path.into() },
                }
            } else {
                InstallMode::OnlineBuild
            };

            let use_native = mode == "native";
            let actual_port = if port == 0 {
                let config = ConfigManager::load()
                    .or_else(|_| ConfigManager::create_default(UserMode::General))?;
                install::next_available_port(&config, desc.default_port)
            } else {
                port
            };

            // Developer mode: --step <name> runs a single step
            if let Some(step_name) = step {
                run_install_step(
                    out, &step_name, &name, &claw_type, &version,
                    use_native, actual_port, api_key.as_deref(), browser,
                ).await?;
            } else {
                // Normal user flow: full install
                let opts = InstallOptions {
                    instance_name: name.clone(),
                    claw_type: claw_type.clone(),
                    claw_version: version,
                    install_mode,
                    install_browser: browser,
                    install_mcp_bridge: desc.supports_mcp,
                    api_key,
                    use_native,
                    gateway_port: actual_port,
                };

                let mut config = ConfigManager::load()
                    .or_else(|_| ConfigManager::create_default(UserMode::General))?;

                let (tx, mut rx) = tokio::sync::mpsc::channel::<install::InstallProgress>(32);

                let out_clone = out.clone();
                let print_task = tokio::spawn(async move {
                    while let Some(progress) = rx.recv().await {
                        out_clone.emit(CliEvent::Progress {
                            stage: serde_json::to_value(&progress.stage).unwrap_or_default()
                                .as_str().unwrap_or("unknown").to_string(),
                            percent: progress.percent,
                            message: progress.message,
                        });
                    }
                });

                out.emit(CliEvent::Info { message: format!("Installing {} ({})...", desc.display_name, claw_type) });
                install::install(opts, &mut config, tx).await?;
                print_task.await?;
                out.emit(CliEvent::Complete { message: format!("{} installed successfully", desc.display_name) });
            }
        }

        // ====== Uninstall ======
        Commands::Uninstall { name } => {
            let mut config = ConfigManager::load()?;
            clawenv_core::manager::instance::remove_instance(&mut config, &name).await?;
            out.emit(CliEvent::Complete { message: format!("Instance '{name}' removed") });
        }

        // ====== List ======
        Commands::List => {
            match ConfigManager::load() {
                Ok(config) => {
                    let mut instances = Vec::new();
                    for inst in config.instances() {
                        let health = clawenv_core::manager::instance::instance_health(inst).await;
                        instances.push(InstanceSummary {
                            name: inst.name.clone(),
                            claw_type: inst.claw_type.clone(),
                            version: inst.version.clone(),
                            sandbox_type: inst.sandbox_type.display_name().to_string(),
                            health: serde_json::to_value(health).unwrap_or_default().as_str().unwrap_or("unreachable").to_string(),
                            gateway_port: inst.gateway.gateway_port,
                            ttyd_port: inst.gateway.ttyd_port,
                            dashboard_port: inst.gateway.dashboard_port,
                        });
                    }
                    let resp = ListResponse { instances };
                    out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
                }
                Err(_) => {
                    let resp = ListResponse { instances: vec![] };
                    out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
                }
            }
        }

        // ====== Start ======
        Commands::Start { name } => {
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            out.emit(CliEvent::Info { message: format!("Starting '{name}'...") });
            clawenv_core::manager::instance::start_instance(inst).await?;
            out.emit(CliEvent::Complete { message: format!("Instance '{name}' started") });
        }

        // ====== Stop ======
        Commands::Stop { name } => {
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            clawenv_core::manager::instance::stop_instance(inst).await?;
            out.emit(CliEvent::Complete { message: format!("Instance '{name}' stopped") });
        }

        // ====== Restart ======
        Commands::Restart { name } => {
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            clawenv_core::manager::instance::restart_instance(inst).await?;
            out.emit(CliEvent::Complete { message: format!("Instance '{name}' restarted") });
        }

        // ====== Status ======
        Commands::Status { name } => {
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let health = clawenv_core::manager::instance::instance_health(inst).await;
            let resp = StatusResponse {
                name: inst.name.clone(),
                claw_type: inst.claw_type.clone(),
                version: inst.version.clone(),
                sandbox_type: inst.sandbox_type.display_name().to_string(),
                health: serde_json::to_value(health).unwrap_or_default().as_str().unwrap_or("unreachable").to_string(),
                gateway_port: inst.gateway.gateway_port,
                ttyd_port: inst.gateway.ttyd_port,
                dashboard_port: inst.gateway.dashboard_port,
                capabilities: None,
                gateway_token: None,
            };
            out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
        }

        // ====== Logs ======
        Commands::Logs { name, follow } => {
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let claw_reg = clawenv_core::claw::ClawRegistry::load();
            let desc = claw_reg.get(&inst.claw_type);
            let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
            let cmd = if follow {
                format!("{} logs -f", desc.cli_binary)
            } else {
                "cat /tmp/clawenv-gateway.log 2>/dev/null | tail -200".to_string()
            };
            let output = backend.exec(&cmd).await?;
            out.emit(CliEvent::Data { data: serde_json::Value::String(output) });
        }

        // ====== Upgrade ======
        Commands::Upgrade { name, version } => {
            let name = resolve_name(name);
            let mut config = ConfigManager::load()?;
            out.emit(CliEvent::Info { message: format!("Upgrading '{name}'...") });

            let (tx, mut rx) = tokio::sync::mpsc::channel::<clawenv_core::manager::upgrade::UpgradeProgress>(16);
            let out_clone = out.clone();
            let print_task = tokio::spawn(async move {
                while let Some(progress) = rx.recv().await {
                    out_clone.emit(CliEvent::Progress {
                        stage: progress.stage,
                        percent: progress.percent,
                        message: progress.message,
                    });
                }
            });

            let new_ver = clawenv_core::manager::upgrade::upgrade_instance(
                &mut config, &name, version.as_deref(), &tx,
            ).await?;
            drop(tx);
            print_task.await?;
            out.emit(CliEvent::Complete { message: format!("Upgraded to {new_ver}") });
        }

        // ====== UpdateCheck ======
        Commands::UpdateCheck { name } => {
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let registry_url = config.config().clawenv.mirrors.npm_registry_url().to_string();
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;

            match clawenv_core::manager::upgrade::check_upgrade(inst, &registry_url).await {
                Ok(info) => {
                    let resp = UpdateCheckResponse {
                        current: info.current,
                        latest: info.latest,
                        has_upgrade: info.has_upgrade,
                        is_security_release: info.is_security_release,
                        changelog: info.changelog,
                    };
                    out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
                }
                Err(e) => anyhow::bail!("Failed to check updates: {e}"),
            }
        }

        // ====== Export ======
        //
        // Stage convention (emitted as CliEvent::Progress so GUI shells can
        // render a staged progress bar without duplicating the export
        // business logic). Stages are in temporal order; percent is a
        // coarse indicator, not a precise byte/file ratio:
        //   `stop`     0→10   stopping the instance quiescent for tar
        //   `compress` 10→90  running tar / podman save / wsl --export
        //   `wrap`     90→95  outer-tar the payload + manifest (Podman/WSL only)
        //   `checksum` 95→99  sizing the output / optional SHA256
        //   `restart`  99→100 bringing the gateway back up
        // The Tauri GUI's export-progress event mirrors these stage names.
        Commands::Export { name, output } => {
            use clawenv_core::sandbox::SandboxType;
            use clawenv_core::export::BundleManifest;
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
            let claw_reg = clawenv_core::claw::ClawRegistry::load();
            let desc = claw_reg.get(&inst.claw_type);

            out.emit(CliEvent::Info { message: format!("Exporting '{name}'...") });
            let version = backend.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await.unwrap_or_default();
            out.emit(CliEvent::Info { message: format!("{}: {}", desc.display_name, version.trim()) });

            // Build the manifest once up-front; each backend decides where
            // to drop it (in-tree for Native/Lima, inside the outer wrap for
            // Podman/WSL). Using the registry claw_type here is what lets
            // the import side drop the old "probe version_check_cmd for
            // every known claw" loop.
            let manifest = BundleManifest::build(
                &inst.claw_type,
                version.trim(),
                inst.sandbox_type.as_wire_str(),
            );

            // Route by backend. Previously this unconditionally called the
            // Alpine packaging script via `bash`, which (a) doesn't know how
            // to package Native-mode installs and (b) on Windows dispatches
            // to `bash.exe` → WSL and fails with an opaque "WSL not
            // installed" error. Native mode instead tars the private
            // ~/.clawenv/{node,git,native} tree directly.
            match inst.sandbox_type {
                SandboxType::Native => {
                    let home = dirs::home_dir()
                        .ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
                    let clawenv = home.join(".clawenv");

                    // Enforce the bundle-self-containment rule: both node
                    // and git must be privately installed. Otherwise the
                    // tarball is useless on machines without system node/git.
                    for sub in ["node", "git", "native"] {
                        if !clawenv.join(sub).exists() {
                            anyhow::bail!(
                                "Cannot export native bundle: ~/.clawenv/{} is missing. \
                                 Re-run the installer to make sure node + git + native \
                                 are all privately installed.",
                                sub
                            );
                        }
                    }

                    let out_path = validate_export_out_path(&output)?;

                    out.emit(CliEvent::Progress {
                        stage: "stop".into(), percent: 5,
                        message: "Stopping native gateway...".into(),
                    });
                    backend.stop().await.ok();
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                    // Drop the manifest next to node/git/native so the
                    // receiver sees `clawenv-bundle.toml` at archive root.
                    // Clean it up after tar to keep ~/.clawenv tidy if the
                    // user ever inspects it.
                    manifest.write_to_dir(&clawenv)?;

                    out.emit(CliEvent::Progress {
                        stage: "compress".into(), percent: 15,
                        message: "Compressing node + git + native tree...".into(),
                    });
                    // Windows' built-in tar.exe (BSD tar + gzip). `-C clawenv`
                    // so archive paths are "node/..", "git/..", "native/.."
                    // and a receiving machine can untar directly into its own
                    // ~/.clawenv/ to restore the bundle.
                    let status = tokio::process::Command::new("tar")
                        .args(["czf",
                               &out_path.to_string_lossy(),
                               "-C", &clawenv.to_string_lossy(),
                               clawenv_core::export::manifest::MANIFEST_FILENAME,
                               "node", "git", "native"])
                        .status().await;

                    // Remove the manifest sidecar once it's in the archive.
                    let _ = std::fs::remove_file(
                        clawenv.join(clawenv_core::export::manifest::MANIFEST_FILENAME),
                    );

                    // Always try to restart the gateway, even on export failure
                    out.emit(CliEvent::Progress {
                        stage: "restart".into(), percent: 99,
                        message: "Restarting native gateway...".into(),
                    });
                    backend.start().await.ok();

                    match status {
                        Ok(s) if s.success() => {
                            out.emit(CliEvent::Complete {
                                message: format!("Exported to {}", out_path.display())
                            });
                        }
                        Ok(s) => anyhow::bail!("tar exited with status {:?}", s.code()),
                        Err(e) => anyhow::bail!("failed to run tar: {e}"),
                    }
                }
                SandboxType::LimaAlpine => {
                    // Lima VM export: tar ~/.clawenv/lima/<sandbox_id>/ (the
                    // private LIMA_HOME tree set up by init_lima_env). The old
                    // path called `bash tools/package-alpine.sh` which
                    // hardcoded ~/.lima and assumed vm_name == "clawenv-<name>"
                    // — both wrong after v0.2.5's LIMA_HOME privatisation and
                    // sandbox_id (auto-generated hash) mapping. Matching the
                    // Native branch's pattern avoids shelling out entirely.
                    let out_path = validate_export_out_path(&output)?;

                    out.emit(CliEvent::Progress {
                        stage: "stop".into(), percent: 5,
                        message: "Stopping Lima VM...".into(),
                    });
                    backend.stop().await.ok();
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                    let lima_home = clawenv_core::sandbox::lima_home();
                    let vm_name = &inst.sandbox_id;

                    // Manifest lives alongside the VM dir inside LIMA_HOME so
                    // it ends up at archive root (tar is `-C lima_home`, then
                    // items `clawenv-bundle.toml` + `<vm_name>/`).
                    manifest.write_to_dir(&lima_home)?;

                    out.emit(CliEvent::Progress {
                        stage: "compress".into(), percent: 15,
                        message: "Compressing Lima VM tree...".into(),
                    });
                    // Exclude runtime-only artefacts so the tarball doesn't
                    // carry dead sockets / stale pids into the receiving
                    // machine. cidata.iso IS included (cloud-init seed for
                    // first-boot provisioning on the target).
                    let status = tokio::process::Command::new("tar")
                        .args([
                            "czf",
                            &out_path.to_string_lossy(),
                            "-C", &lima_home.to_string_lossy(),
                            "--exclude", &format!("{vm_name}/*.sock"),
                            "--exclude", &format!("{vm_name}/*.pid"),
                            "--exclude", &format!("{vm_name}/*.log"),
                            clawenv_core::export::manifest::MANIFEST_FILENAME,
                            vm_name,
                        ])
                        .status().await;

                    let _ = std::fs::remove_file(
                        lima_home.join(clawenv_core::export::manifest::MANIFEST_FILENAME),
                    );

                    out.emit(CliEvent::Progress {
                        stage: "restart".into(), percent: 99,
                        message: "Restarting Lima VM...".into(),
                    });
                    backend.start().await.ok();

                    match status {
                        Ok(s) if s.success() => {
                            out.emit(CliEvent::Complete {
                                message: format!("Exported to {}", out_path.display())
                            });
                        }
                        Ok(s) => anyhow::bail!("tar exited with status {:?}", s.code()),
                        Err(e) => anyhow::bail!("failed to run tar: {e}"),
                    }
                }
                SandboxType::PodmanAlpine => {
                    // Podman: commit running container to image, then
                    // `podman save` to tarball. The PodmanBackend's XDG env
                    // vars are already set at process start by init_podman_env.
                    let out_path = validate_export_out_path(&output)?;

                    let vm_name = &inst.sandbox_id;
                    let image_tag = format!("clawenv-export:{name}");
                    out.emit(CliEvent::Progress {
                        stage: "compress".into(), percent: 15,
                        message: "Committing Podman container...".into(),
                    });
                    let commit = tokio::process::Command::new("podman")
                        .args(["commit", vm_name, &image_tag])
                        .status().await?;
                    if !commit.success() {
                        anyhow::bail!("podman commit failed");
                    }

                    // podman save produces an image tarball that's
                    // inherently not a filesystem tar, so we can't stuff the
                    // manifest inside alongside the image layers. Instead we
                    // save to a temp tarball and wrap it in an outer tar.gz
                    // together with the manifest; the import side unwraps
                    // `payload.tar` back out before `podman load -i`.
                    let parent = out_path.parent().unwrap_or(std::path::Path::new("."));
                    let inner_path = parent.join(format!(
                        ".clawenv-podman-save-{}.tar", std::process::id()
                    ));
                    out.emit(CliEvent::Progress {
                        stage: "compress".into(), percent: 40,
                        message: "podman save (image → tar)...".into(),
                    });
                    let save = tokio::process::Command::new("podman")
                        .args(["save", "-o", &inner_path.to_string_lossy(), &image_tag])
                        .status().await?;
                    if !save.success() {
                        let _ = std::fs::remove_file(&inner_path);
                        anyhow::bail!("podman save failed");
                    }

                    out.emit(CliEvent::Progress {
                        stage: "wrap".into(), percent: 90,
                        message: "Wrapping payload + manifest...".into(),
                    });
                    let wrap_result = manifest.wrap_with_inner_tar(&inner_path, &out_path).await;
                    // wrap_with_inner_tar renames/copies the inner in — clean
                    // up any leftover if wrap bailed mid-flight.
                    let _ = std::fs::remove_file(&inner_path);
                    wrap_result?;

                    out.emit(CliEvent::Complete {
                        message: format!("Exported to {}", out_path.display())
                    });
                }
                SandboxType::Wsl2Alpine => {
                    // WSL: `wsl --export <distro> <file>` is the native path.
                    // The distro data already lives in ~/.clawenv/wsl/ from
                    // install time (WslBackend was always private).
                    let out_path = validate_export_out_path(&output)?;

                    let vm_name = &inst.sandbox_id;

                    // Same wrap pattern as Podman: `wsl --export` produces
                    // a distro tarball that isn't a filesystem tar we can
                    // append to, so we write it to a temp file first, then
                    // wrap it + the manifest into the user-facing tar.gz.
                    let parent = out_path.parent().unwrap_or(std::path::Path::new("."));
                    let inner_path = parent.join(format!(
                        ".clawenv-wsl-export-{}.tar", std::process::id()
                    ));
                    out.emit(CliEvent::Progress {
                        stage: "compress".into(), percent: 20,
                        message: "wsl --export (distro → tar)...".into(),
                    });
                    let status = tokio::process::Command::new("wsl")
                        .args(["--export", vm_name, &inner_path.to_string_lossy()])
                        .status().await?;
                    if !status.success() {
                        let _ = std::fs::remove_file(&inner_path);
                        anyhow::bail!("wsl --export failed");
                    }

                    out.emit(CliEvent::Progress {
                        stage: "wrap".into(), percent: 90,
                        message: "Wrapping payload + manifest...".into(),
                    });
                    let wrap_result = manifest.wrap_with_inner_tar(&inner_path, &out_path).await;
                    let _ = std::fs::remove_file(&inner_path);
                    wrap_result?;

                    out.emit(CliEvent::Complete {
                        message: format!("Exported to {}", out_path.display())
                    });
                }
            }
        }

        // ====== Import ======
        Commands::Import { file, name } => {
            use clawenv_core::config::{InstanceConfig, GatewayConfig, ResourceConfig};
            use clawenv_core::export::BundleManifest;
            use clawenv_core::sandbox::SandboxType;

            let path = std::path::PathBuf::from(&file);
            if !path.exists() {
                anyhow::bail!("File not found: {}", path.display());
            }

            // Peek the bundle manifest FIRST. Manifests became mandatory in
            // v0.2.6 — bundles produced by earlier clawenv don't carry one
            // and are rejected outright here (by explicit user decision; the
            // compat shim was dropped). This also lets us validate
            // host-vs-source sandbox type and claw_type before we spend any
            // time untarring.
            let manifest = BundleManifest::peek_from_tarball(&path).await
                .map_err(|e| anyhow::anyhow!(
                    "Cannot import {}: {e}\n\nThis version of clawenv only imports \
                     bundles produced by clawenv v0.2.6 or later. Re-export from the \
                     source machine with a current clawenv build.",
                    path.display()
                ))?;

            let host_sandbox = SandboxType::from_os();
            let bundle_sandbox = SandboxType::parse_wire(&manifest.sandbox_type)
                .ok_or_else(|| anyhow::anyhow!(
                    "Bundle declares unknown sandbox_type '{}'", manifest.sandbox_type
                ))?;
            if bundle_sandbox != host_sandbox {
                anyhow::bail!(
                    "Cannot import {}: bundle was produced for sandbox '{}' but this host \
                     uses '{}'. Cross-backend import is not supported — run the bundle \
                     on a matching OS/backend.",
                    path.display(),
                    bundle_sandbox.as_wire_str(),
                    host_sandbox.as_wire_str(),
                );
            }

            out.emit(CliEvent::Info {
                message: format!(
                    "Importing '{name}' ({} / {}) from {file}...",
                    manifest.claw_type, manifest.sandbox_type
                )
            });
            let backend = clawenv_core::sandbox::detect_backend_for(&name)?;
            backend.import_image(&path).await?;

            // Claw identity comes from the manifest — no more probe loop.
            // The version the source reported is authoritative; if the user
            // wants a fresh reading they can hit `clawenv list --refresh`.
            let claw_type = manifest.claw_type.clone();
            let claw_version = manifest.claw_version.clone();

            // Save instance config
            let mut config = ConfigManager::load()
                .or_else(|_| ConfigManager::create_default(UserMode::General))?;
            // Look up the claw descriptor to decide whether to provision
            // a dashboard_port. Imported instances land on the fixed
            // 3000-block (no multi-instance conflict management — import
            // is always into a fresh slot) so we compute offsets
            // statically rather than going through allocate_port.
            let claw_reg_for_import = clawenv_core::claw::ClawRegistry::load();
            let desc_for_import = claw_reg_for_import.get(&claw_type);
            let gateway_port = 3000u16;
            let dashboard_port = if desc_for_import.has_dashboard() {
                gateway_port + desc_for_import.dashboard_port_offset
            } else { 0 };
            config.save_instance(InstanceConfig {
                name: name.clone(),
                claw_type,
                version: claw_version,
                sandbox_type: host_sandbox,
                sandbox_id: format!("clawenv-{name}"),
                created_at: chrono::Utc::now().to_rfc3339(),
                last_upgraded_at: String::new(),
                gateway: GatewayConfig {
                    gateway_port,
                    ttyd_port: gateway_port + 1,
                    bridge_port: gateway_port + 2,
                    dashboard_port,
                    webchat_enabled: true,
                    channels: Default::default(),
                },
                resources: ResourceConfig::default(),
                browser: Default::default(),
                cached_latest_version: String::new(),
                cached_version_check_at: String::new(),
            })?;

            out.emit(CliEvent::Complete { message: format!("Imported '{name}'. Use 'clawenv start {name}' to start.") });
        }

        // ====== Doctor ======
        Commands::Doctor => {
            let platform = clawenv_core::platform::detect_platform()?;
            let memory = clawenv_core::platform::process::system_memory_gb().await;
            let disk = clawenv_core::platform::process::disk_free_gb().await;

            let (backend_name, backend_available) = match clawenv_core::sandbox::detect_backend() {
                Ok(b) => {
                    let avail = b.is_available().await.unwrap_or(false);
                    (b.name().to_string(), avail)
                }
                Err(e) => (format!("error: {e}"), false),
            };

            let instance_count = ConfigManager::load()
                .map(|c| c.instances().len())
                .unwrap_or(0);

            let resp = DoctorResponse {
                os: format!("{:?}", platform.os),
                arch: format!("{:?}", platform.arch),
                memory_gb: format!("{:.1}", memory),
                disk_free_gb: format!("{:.0}", disk),
                sandbox_backend: backend_name,
                sandbox_available: backend_available,
                instances: instance_count,
            };
            out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
        }

        // ====== Exec ======
        Commands::Exec { cmd, name } => {
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
            let output = backend.exec(&cmd).await?;
            print!("{output}");
        }

        // ====== ClawTypes ======
        Commands::ClawTypes => {
            let registry = clawenv_core::claw::ClawRegistry::load();
            let types: Vec<ClawTypeInfo> = registry.list_all().iter().map(|d| ClawTypeInfo {
                id: d.id.clone(),
                display_name: d.display_name.clone(),
                logo: d.logo.clone(),
                package_manager: match d.package_manager {
                    clawenv_core::claw::descriptor::PackageManager::Npm => "npm",
                    clawenv_core::claw::descriptor::PackageManager::Pip => "pip",
                    clawenv_core::claw::descriptor::PackageManager::GitPip => "git_pip",
                }.to_string(),
                npm_package: d.npm_package.clone(),
                pip_package: d.pip_package.clone(),
                default_port: d.default_port,
                supports_mcp: d.supports_mcp,
                supports_browser: d.supports_browser,
                has_gateway_ui: d.has_gateway_ui,
                supports_native: d.supports_native,
            }).collect();
            let resp = ClawTypesResponse { claw_types: types };
            out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
        }

        // ====== SystemCheck ======
        Commands::SystemCheck => {
            let platform = clawenv_core::platform::detect_platform()?;
            let memory = clawenv_core::platform::process::system_memory_gb().await;
            let disk = clawenv_core::platform::process::disk_free_gb().await;

            let (backend_name, backend_available) = match clawenv_core::sandbox::detect_backend() {
                Ok(b) => {
                    let avail = b.is_available().await.unwrap_or(false);
                    (b.name().to_string(), avail)
                }
                Err(e) => (format!("error: {e}"), false),
            };

            #[allow(unused_mut)]
            let mut checks = vec![
                CheckItem { name: "OS".into(), ok: true, detail: format!("{:?} ({:?})", platform.os, platform.arch), info_only: false },
                CheckItem { name: "Memory".into(), ok: memory >= 2.0, detail: format!("{:.1} GB", memory), info_only: false },
                CheckItem { name: "Disk".into(), ok: disk >= 2.0, detail: format!("{:.0} GB free", disk), info_only: false },
                CheckItem { name: "Sandbox".into(), ok: backend_available, detail: backend_name.clone(), info_only: !backend_available },
            ];

            // Cross-platform proxy detection: check config + env vars
            {
                let proxy_detail = if let Ok(cfg) = ConfigManager::load() {
                    let p = &cfg.config().clawenv.proxy;
                    if p.enabled && !p.http_proxy.is_empty() {
                        format!("Config: {}", p.http_proxy)
                    } else if let Ok(env_proxy) = std::env::var("http_proxy").or_else(|_| std::env::var("HTTP_PROXY")) {
                        format!("Env: {env_proxy}")
                    } else {
                        "None".into()
                    }
                } else if let Ok(env_proxy) = std::env::var("http_proxy").or_else(|_| std::env::var("HTTP_PROXY")) {
                    format!("Env: {env_proxy}")
                } else {
                    "None".into()
                };
                let has_proxy = proxy_detail != "None";
                checks.push(CheckItem {
                    name: "Proxy".into(),
                    ok: true,
                    detail: proxy_detail,
                    info_only: !has_proxy,
                });
            }

            let resp = SystemCheckResponse {
                os: format!("{:?}", platform.os),
                arch: format!("{:?}", platform.arch),
                memory_gb: memory,
                disk_free_gb: disk,
                sandbox_backend: backend_name,
                sandbox_available: backend_available,
                checks,
            };
            out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
        }

        // ====== Rename ======
        Commands::Rename { old_name, new_name } => {
            let mut config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &old_name)?.clone();
            let backend = clawenv_core::manager::instance::backend_for_instance(&inst)?;

            out.emit(CliEvent::Info { message: format!("Renaming '{old_name}' → '{new_name}'...") });
            if let Err(e) = clawenv_core::manager::instance::stop_instance(&inst).await {
                out.emit(CliEvent::Info { message: format!("Warning: could not stop instance: {e}") });
            }

            let new_sandbox_id = if backend.supports_rename() {
                backend.rename(&new_name).await?
            } else {
                format!("{:?}-{}", inst.sandbox_type, new_name).to_lowercase()
            };

            let nn = new_name.clone();
            config.update_instance(&old_name, move |entry| {
                entry.name = nn;
                entry.sandbox_id = new_sandbox_id;
            })?;

            let home = dirs::home_dir().unwrap_or_default();
            let old_ws = home.join(format!(".clawenv/workspaces/{old_name}"));
            let new_ws = home.join(format!(".clawenv/workspaces/{new_name}"));
            if old_ws.exists() {
                tokio::fs::rename(&old_ws, &new_ws).await.ok();
            }

            out.emit(CliEvent::Complete { message: format!("Renamed '{old_name}' → '{new_name}'") });
        }

        // ====== Edit ======
        Commands::Edit { name, cpus, memory, disk, gateway_port, ttyd_port } => {
            let mut config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;

            // Edit backend resources if any resource flags provided
            if cpus.is_some() || memory.is_some() || disk.is_some() {
                let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
                if !backend.supports_resource_edit() {
                    anyhow::bail!("Backend {:?} does not support resource editing", inst.sandbox_type);
                }
                out.emit(CliEvent::Info { message: format!("Stopping '{name}' for resource edit...") });
                if let Err(e) = clawenv_core::manager::instance::stop_instance(inst).await {
                    out.emit(CliEvent::Info { message: format!("Warning: could not stop instance: {e}") });
                }
                backend.edit_resources(cpus, memory, disk).await?;
                out.emit(CliEvent::Info { message: "Resources updated".into() });
            }

            // Edit ports if any port flags provided
            if gateway_port.is_some() || ttyd_port.is_some() {
                let gp = gateway_port.unwrap_or(inst.gateway.gateway_port);
                let tp = ttyd_port.unwrap_or(inst.gateway.ttyd_port);

                // Validate port uniqueness
                if let Some(new_port) = gateway_port {
                    clawenv_core::manager::install::validate_port_available(&config, &name, new_port)?;
                }

                let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
                if backend.supports_port_edit() {
                    if let Err(e) = clawenv_core::manager::instance::stop_instance(inst).await {
                        out.emit(CliEvent::Info { message: format!("Warning: could not stop instance: {e}") });
                    }
                    backend.edit_port_forwards(&[(gp, gp), (tp, tp)]).await?;
                }

                config.update_instance(&name, |entry| {
                    entry.gateway.gateway_port = gp;
                    entry.gateway.ttyd_port = tp;
                })?;
                out.emit(CliEvent::Info { message: format!("Ports updated: gateway={gp}, ttyd={tp}") });
            }

            out.emit(CliEvent::Complete { message: format!("Instance '{name}' updated") });
        }

        // ====== Sandbox ======
        Commands::Sandbox(subcmd) => {
            match subcmd {
                SandboxCmd::List => {
                    let mut vms = Vec::new();

                    #[cfg(target_os = "macos")]
                    {
                        let output = tokio::process::Command::new(clawenv_core::sandbox::limactl_bin())
                            .args(["list", "--format", "{{.Name}}\t{{.Status}}\t{{.CPUs}}\t{{.Memory}}\t{{.Disk}}"])
                            .output().await;
                        if let Ok(o) = output {
                            for line in String::from_utf8_lossy(&o.stdout).lines() {
                                let p: Vec<&str> = line.split('\t').collect();
                                if p.len() >= 5 {
                                    vms.push(SandboxVmInfo {
                                        name: p[0].into(), status: p[1].into(),
                                        cpus: p[2].into(), memory: p[3].into(),
                                        disk: p[4].into(), dir_size: "-".into(),
                                        managed: p[0].starts_with("clawenv-"),
                                        ttyd_port: None,
                                        instance_name: None,
                                    });
                                }
                            }
                        }
                    }

                    #[cfg(target_os = "linux")]
                    {
                        let output = tokio::process::Command::new("podman")
                            .args(["ps", "-a", "--format", "{{.Names}}\t{{.Status}}\t{{.Size}}"])
                            .output().await;
                        if let Ok(o) = output {
                            for line in String::from_utf8_lossy(&o.stdout).lines() {
                                let p: Vec<&str> = line.split('\t').collect();
                                if !p.is_empty() {
                                    vms.push(SandboxVmInfo {
                                        name: p[0].into(), status: p.get(1).unwrap_or(&"").to_string(),
                                        cpus: "-".into(), memory: "-".into(),
                                        disk: p.get(2).unwrap_or(&"-").to_string(), dir_size: "-".into(),
                                        managed: p[0].starts_with("clawenv-"),
                                        ttyd_port: None,
                                        instance_name: None,
                                    });
                                }
                            }
                        }
                    }

                    #[cfg(target_os = "windows")]
                    {
                        // WSL outputs UTF-16LE on Windows. Use --list --quiet first to check
                        // if any distros exist (returns just names, one per line).
                        let has_distros = clawenv_core::platform::process::silent_cmd("wsl")
                            .args(["--list", "--quiet"])
                            .output().await
                            .map(|o| {
                                // Decode UTF-16LE, check if any non-empty line exists
                                let u16s: Vec<u16> = o.stdout.chunks_exact(2)
                                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                    .collect();
                                let text = String::from_utf16_lossy(&u16s);
                                o.status.success() && text.lines().any(|l| !l.trim().is_empty() && l.trim().chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                            }).unwrap_or(false);

                        if has_distros {
                            let output = clawenv_core::platform::process::silent_cmd("wsl")
                                .args(["--list", "--verbose"])
                                .output().await;
                            if let Ok(o) = output {
                                // WSL outputs UTF-16LE; decode it
                                let text = String::from_utf8(o.stdout.clone())
                                    .or_else(|_| {
                                        // Decode UTF-16LE
                                        let u16s: Vec<u16> = o.stdout.chunks_exact(2)
                                            .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                            .collect();
                                        Ok::<String, std::string::FromUtf8Error>(String::from_utf16_lossy(&u16s))
                                    })
                                    .unwrap_or_default();

                                for line in text.lines().skip(1) {
                                    let p: Vec<&str> = line.split_whitespace().collect();
                                    if p.len() >= 3 {
                                        let name = p[0].trim_start_matches('*').trim();
                                        if name.is_empty() { continue; }
                                        vms.push(SandboxVmInfo {
                                            name: name.into(), status: p[1].into(),
                                            cpus: "-".into(), memory: "-".into(),
                                            disk: "-".into(), dir_size: "-".into(),
                                            managed: name.starts_with("ClawEnv") || name.starts_with("clawenv"),
                                            ttyd_port: None,
                                            instance_name: None,
                                        });
                                    }
                                }
                            }
                        }
                        // If WSL not installed, vms stays empty — no phantom entries
                    }

                    // Fill ttyd_port AND instance_name for managed VMs by matching
                    // VM name against each instance's sandbox_id (= VM name). The
                    // old code stripped "clawenv-" and matched by instance.name,
                    // which silently failed because sandbox_id contains an auto-
                    // generated hash, not the user-chosen name.
                    let config = ConfigManager::load()?;
                    for vm in &mut vms {
                        if vm.managed {
                            if let Some(inst) = config.instances().iter().find(|i| i.sandbox_id == vm.name) {
                                vm.ttyd_port = Some(inst.gateway.ttyd_port);
                                vm.instance_name = Some(inst.name.clone());
                            }
                        }
                    }

                    let resp = SandboxListResponse {
                        total_disk_usage: "-".into(),
                        vms,
                    };
                    out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
                }

                SandboxCmd::Info => {
                    let disk = clawenv_core::platform::process::disk_free_gb().await;
                    let (backend_name, backend_available) = match clawenv_core::sandbox::detect_backend() {
                        Ok(b) => (b.name().to_string(), b.is_available().await.unwrap_or(false)),
                        Err(e) => (format!("error: {e}"), false),
                    };
                    let resp = DoctorResponse {
                        os: String::new(),
                        arch: String::new(),
                        memory_gb: String::new(),
                        disk_free_gb: format!("{:.0}", disk),
                        sandbox_backend: backend_name,
                        sandbox_available: backend_available,
                        instances: 0,
                    };
                    out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
                }

                SandboxCmd::Shell { name } => {
                    let name = resolve_name(name);
                    let config = ConfigManager::load()?;
                    let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;

                    // Launch interactive shell — must use std::process (not tokio) to inherit stdio
                    let status = match inst.sandbox_type {
                        clawenv_core::sandbox::SandboxType::LimaAlpine => {
                            std::process::Command::new(clawenv_core::sandbox::limactl_bin())
                                .args(["shell", &format!("clawenv-{name}")])
                                .status()?
                        }
                        clawenv_core::sandbox::SandboxType::Wsl2Alpine => {
                            std::process::Command::new("wsl")
                                .args(["-d", &format!("ClawEnv-{name}")])
                                .status()?
                        }
                        clawenv_core::sandbox::SandboxType::PodmanAlpine => {
                            std::process::Command::new("podman")
                                .args(["exec", "-it", &format!("clawenv-{name}"), "/bin/sh"])
                                .status()?
                        }
                        clawenv_core::sandbox::SandboxType::Native => {
                            anyhow::bail!("Native instances have no sandbox shell. Use your terminal directly.");
                        }
                    };

                    if !status.success() {
                        anyhow::bail!("Shell exited with code {:?}", status.code());
                    }
                }
            }
        }

        // ====== Config ======
        Commands::Config(subcmd) => {
            match subcmd {
                ConfigCmd::Show => {
                    let config = ConfigManager::load()
                        .or_else(|_| ConfigManager::create_default(UserMode::General))?;
                    let c = config.config();
                    let resp = ConfigShowResponse {
                        language: c.clawenv.language.clone(),
                        theme: c.clawenv.theme.clone(),
                        user_mode: format!("{:?}", c.clawenv.user_mode),
                        proxy_enabled: c.clawenv.proxy.enabled,
                        proxy_http: c.clawenv.proxy.http_proxy.clone(),
                        proxy_https: c.clawenv.proxy.https_proxy.clone(),
                        proxy_no_proxy: c.clawenv.proxy.no_proxy.clone(),
                        mirrors_preset: c.clawenv.mirrors.preset.clone(),
                        bridge_enabled: c.clawenv.bridge.enabled,
                        bridge_port: c.clawenv.bridge.port,
                        updates_auto_check: c.clawenv.updates.auto_check,
                        instances_count: c.instances.len(),
                    };
                    out.emit(CliEvent::Data { data: serde_json::to_value(&resp)? });
                }

                ConfigCmd::Set { key, value } => {
                    let display_value = value.clone();
                    let mut config = ConfigManager::load()
                        .or_else(|_| ConfigManager::create_default(UserMode::General))?;
                    let c = config.config_mut();

                    match key.as_str() {
                        "language" => c.clawenv.language = value,
                        "theme" => c.clawenv.theme = value,
                        "proxy.enabled" => c.clawenv.proxy.enabled = value.parse().unwrap_or(false),
                        "proxy.http" => c.clawenv.proxy.http_proxy = value,
                        "proxy.https" => c.clawenv.proxy.https_proxy = value,
                        "proxy.no_proxy" => c.clawenv.proxy.no_proxy = value,
                        "mirrors.preset" => c.clawenv.mirrors.preset = value,
                        "bridge.enabled" => c.clawenv.bridge.enabled = value.parse().unwrap_or(true),
                        "bridge.port" => c.clawenv.bridge.port = value.parse().unwrap_or(3100),
                        "updates.auto_check" => c.clawenv.updates.auto_check = value.parse().unwrap_or(true),
                        _ => anyhow::bail!("Unknown config key: '{key}'. Valid keys: language, theme, proxy.enabled, proxy.http, proxy.https, proxy.no_proxy, mirrors.preset, bridge.enabled, bridge.port, updates.auto_check"),
                    }

                    config.save()?;
                    out.emit(CliEvent::Complete { message: format!("Config '{key}' set to '{display_value}'") });
                }

                ConfigCmd::ProxyTest => {
                    let config = ConfigManager::load()?;
                    let proxy = &config.config().clawenv.proxy;
                    if !proxy.enabled || proxy.http_proxy.is_empty() {
                        out.emit(CliEvent::Info { message: "No proxy configured".into() });
                        return Ok(());
                    }
                    out.emit(CliEvent::Info { message: format!("Testing proxy {}...", proxy.http_proxy) });
                    clawenv_core::config::proxy::test_proxy(proxy, "").await?;
                    out.emit(CliEvent::Complete { message: "Proxy test passed".into() });
                }
            }
        }

        // ====== Bridge ======
        Commands::Bridge(subcmd) => {
            match subcmd {
                BridgeCmd::Test { port } => {
                    let bridge_port = port.unwrap_or_else(|| {
                        ConfigManager::load()
                            .map(|c| c.config().clawenv.bridge.port)
                            .unwrap_or(3100)
                    });
                    out.emit(CliEvent::Info { message: format!("Testing bridge on port {bridge_port}...") });
                    let url = format!("http://127.0.0.1:{bridge_port}/api/health");
                    let client = reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(5))
                        .build()?;
                    match client.get(&url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            let body = resp.text().await.unwrap_or_default();
                            out.emit(CliEvent::Data { data: serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body)) });
                            out.emit(CliEvent::Complete { message: format!("Bridge is running on port {bridge_port}") });
                        }
                        Ok(resp) => {
                            anyhow::bail!("Bridge responded with HTTP {}", resp.status());
                        }
                        Err(e) => {
                            anyhow::bail!("Bridge not reachable on port {bridge_port}: {e}");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Developer mode: run a single install step.
///
/// Steps:
///   prereq  — Check/install prerequisites (Lima/WSL2/Podman or Node.js)
///   create  — Create sandbox VM or native directory
///   claw    — Install claw product inside existing environment
///   config  — Store API key + save instance config
///   gateway — Start gateway service
///
/// 9 args is over clippy's default threshold, but all of them are distinct
/// install inputs that the wizard collects separately and hands off here.
/// Bundling them into a struct would just rename the args without
/// simplifying anything — the struct would be used exactly once. Silenced
/// with a reason so we don't pretend the lint is unknown.
#[allow(clippy::too_many_arguments)]
async fn run_install_step(
    out: &Output,
    step: &str,
    name: &str,
    claw_type: &str,
    version: &str,
    use_native: bool,
    port: u16,
    api_key: Option<&str>,
    install_browser: bool,
) -> Result<()> {
    use clawenv_core::config::{ConfigManager, UserMode, InstanceConfig, GatewayConfig, ResourceConfig};
    use clawenv_core::sandbox::{detect_backend_for, SandboxBackend, SandboxOpts, SandboxType, InstallMode};
    use clawenv_core::claw::ClawRegistry;

    let registry = ClawRegistry::load();
    let desc = registry.get_strict(claw_type)?;

    match step {
        // ---- Step: prereq ----
        "prereq" => {
            if use_native {
                out.emit(CliEvent::Info { message: "Checking Node.js...".into() });
                if clawenv_core::manager::install_native::has_node().await {
                    out.emit(CliEvent::Complete { message: "Node.js already available".into() });
                } else {
                    out.emit(CliEvent::Info { message: "Node.js not found, installing...".into() });
                    let config = ConfigManager::load()
                        .or_else(|_| ConfigManager::create_default(UserMode::General))?;
                    let mirrors = &config.config().clawenv.mirrors;
                    let (tx, _rx) = tokio::sync::mpsc::channel(8);
                    clawenv_core::manager::install_native::install_nodejs_public(&tx, mirrors.nodejs_dist_url()).await?;
                    out.emit(CliEvent::Complete { message: "Node.js installed".into() });
                }
            } else {
                out.emit(CliEvent::Info { message: "Checking sandbox backend...".into() });
                let backend = detect_backend_for(name)?;
                let available = backend.is_available().await.unwrap_or(false);
                if available {
                    out.emit(CliEvent::Complete { message: format!("{} ready", backend.name()) });
                } else {
                    out.emit(CliEvent::Info { message: format!("Installing {}...", backend.name()) });
                    backend.ensure_prerequisites().await?;
                    out.emit(CliEvent::Complete { message: format!("{} installed", backend.name()) });
                }
            }
        }

        // ---- Step: create ----
        "create" => {
            if use_native {
                let install_dir = dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".clawenv/native")
                    .join(name);
                tokio::fs::create_dir_all(&install_dir).await?;
                out.emit(CliEvent::Info { message: "Ensuring Node.js...".into() });
                if !clawenv_core::manager::install_native::has_node().await {
                    let config = ConfigManager::load()
                        .or_else(|_| ConfigManager::create_default(UserMode::General))?;
                    let mirrors = &config.config().clawenv.mirrors;
                    let (tx, _rx) = tokio::sync::mpsc::channel(8);
                    clawenv_core::manager::install_native::install_nodejs_public(&tx, mirrors.nodejs_dist_url()).await?;
                }
                clawenv_core::manager::install_native::ensure_node_in_path();
                out.emit(CliEvent::Complete { message: format!("Native environment ready at {}", install_dir.display()) });
            } else {
                let backend = detect_backend_for(name)?;
                // Check if VM already exists
                let vm_ready = backend.exec("node --version 2>/dev/null").await
                    .map(|o| o.trim().starts_with('v')).unwrap_or(false);
                if vm_ready {
                    out.emit(CliEvent::Complete { message: "VM already exists and is provisioned".into() });
                } else {
                    let config = ConfigManager::load()
                        .or_else(|_| ConfigManager::create_default(UserMode::General))?;
                    let mirrors = &config.config().clawenv.mirrors;
                    let alpine_mirror = if mirrors.is_default() { String::new() } else { mirrors.alpine_repo_url().to_string() };
                    let npm_registry = if mirrors.is_default() { String::new() } else { mirrors.npm_registry_url().to_string() };
                    let opts = SandboxOpts {
                        instance_name: name.to_string(),
                        claw_type: claw_type.to_string(),
                        claw_version: version.to_string(),
                        alpine_version: "latest-stable".into(),
                        memory_mb: 512,
                        cpu_cores: 2,
                        install_browser,
                        install_mode: InstallMode::OnlineBuild,
                        proxy_script: String::new(),
                        gateway_port: port,
                        alpine_mirror,
                        npm_registry,
                    };
                    out.emit(CliEvent::Info { message: "Creating VM (this takes a few minutes)...".into() });
                    backend.create(&opts).await?;
                    out.emit(CliEvent::Complete { message: "VM created with system packages".into() });
                }
            }
        }

        // ---- Step: claw ----
        "claw" => {
            out.emit(CliEvent::Info { message: format!("Installing {} in '{}'...", desc.display_name, name) });
            if use_native {
                let backend = clawenv_core::sandbox::native_backend(name);
                let already = backend.exec(&format!("{} 2>/dev/null || echo ''", desc.version_check_cmd())).await
                    .map(|o| !o.trim().is_empty()).unwrap_or(false);
                if already {
                    let ver = backend.exec(&desc.version_check_cmd()).await.unwrap_or_default();
                    out.emit(CliEvent::Complete { message: format!("{} {} already installed", desc.display_name, ver.trim()) });
                } else {
                    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
                    let out_clone = out.clone();
                    let _dn = desc.display_name.clone();
                    let ui = tokio::spawn(async move {
                        let start = std::time::Instant::now();
                        while let Some(line) = rx.recv().await {
                            let t = line.trim();
                            if !t.is_empty() {
                                let e = start.elapsed().as_secs();
                                let short = if t.len() > 80 { &t[..80] } else { t };
                                out_clone.emit(CliEvent::Progress {
                                    stage: "InstallClaw".into(),
                                    percent: std::cmp::min(30 + (e / 10) as u8, 90),
                                    message: format!("[{e}s] {short}"),
                                });
                            }
                        }
                    });
                    backend.exec_with_progress(&desc.npm_install_verbose_cmd(version), &tx).await?;
                    drop(tx);
                    ui.await.ok();
                    let ver = backend.exec(&desc.version_check_cmd()).await.unwrap_or_default();
                    out.emit(CliEvent::Complete { message: format!("{} {} installed", desc.display_name, ver.trim()) });
                }
            } else {
                let backend = detect_backend_for(name)?;
                let already = backend.exec(&format!("which {} 2>/dev/null", desc.cli_binary)).await
                    .map(|o| !o.trim().is_empty()).unwrap_or(false);
                if already {
                    let ver = backend.exec(&desc.version_check_cmd()).await.unwrap_or_default();
                    out.emit(CliEvent::Complete { message: format!("{} {} already installed", desc.display_name, ver.trim()) });
                } else {
                    let raw_cmd = desc.npm_install_verbose_cmd(version);
                    // Sandbox: wrap with sudo (non-root user may lack /usr/local/lib write permission)
                    let cmd = format!("sudo {raw_cmd}");
                    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
                    let out_clone = out.clone();
                    let _dn = desc.display_name.clone();
                    let ui = tokio::spawn(async move {
                        let start = std::time::Instant::now();
                        while let Some(line) = rx.recv().await {
                            let t = line.trim();
                            if !t.is_empty() {
                                let e = start.elapsed().as_secs();
                                let short = if t.len() > 80 { &t[..80] } else { t };
                                out_clone.emit(CliEvent::Progress {
                                    stage: "InstallClaw".into(),
                                    percent: std::cmp::min(30 + (e / 10) as u8, 90),
                                    message: format!("[{e}s] {short}"),
                                });
                            }
                        }
                    });
                    backend.exec_with_progress(&cmd, &tx).await?;
                    drop(tx);
                    ui.await.ok();
                    let ver = backend.exec(&desc.version_check_cmd()).await.unwrap_or_default();
                    out.emit(CliEvent::Complete { message: format!("{} {} installed", desc.display_name, ver.trim()) });
                }
            }
        }

        // ---- Step: config ----
        "config" => {
            let mut config = ConfigManager::load()
                .or_else(|_| ConfigManager::create_default(UserMode::General))?;

            // Validate port uniqueness
            clawenv_core::manager::install::validate_port_available(&config, name, port)?;

            // Store API key
            if let Some(key) = api_key {
                out.emit(CliEvent::Info { message: "Storing API key...".into() });
                clawenv_core::config::keychain::store_api_key(name, key)?;
                // Also set in sandbox/native
                if let Some(cmd) = desc.set_apikey_cmd(&clawenv_core::manager::install::shell_escape(key)) {
                    if use_native {
                        let b = clawenv_core::sandbox::native_backend(name);
                        b.exec(&format!("{cmd} 2>/dev/null || true")).await?;
                    } else {
                        let b = detect_backend_for(name)?;
                        b.exec(&format!("{cmd} 2>/dev/null || true")).await?;
                    }
                }
            }

            // Get version
            let claw_version = if use_native {
                let b = clawenv_core::sandbox::native_backend(name);
                b.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await.unwrap_or_default()
            } else {
                let b = detect_backend_for(name)?;
                b.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await.unwrap_or_default()
            };

            let sandbox_type = if use_native { SandboxType::Native } else { SandboxType::from_os() };
            let sandbox_id = if use_native { format!("native-{name}") } else { format!("clawenv-{name}") };
            let ttyd_port = if use_native { 0 } else { port + 4681 };

            config.save_instance(InstanceConfig {
                name: name.to_string(),
                claw_type: claw_type.to_string(),
                version: claw_version.trim().to_string(),
                sandbox_type,
                sandbox_id,
                created_at: chrono::Utc::now().to_rfc3339(),
                last_upgraded_at: String::new(),
                gateway: GatewayConfig {
                    gateway_port: port,
                    ttyd_port,
                    bridge_port: clawenv_core::manager::install::allocate_port(port, 2),
                    dashboard_port: if desc.has_dashboard() {
                        clawenv_core::manager::install::allocate_port(port, desc.dashboard_port_offset)
                    } else { 0 },
                    webchat_enabled: true,
                    channels: Default::default(),
                },
                resources: ResourceConfig::default(),
                browser: Default::default(),
                cached_latest_version: String::new(),
                cached_version_check_at: String::new(),
            })?;
            out.emit(CliEvent::Complete { message: format!("Instance '{}' config saved (port {})", name, port) });
        }

        // ---- Step: gateway ----
        "gateway" => {
            if let Some(gateway_cmd) = desc.gateway_start_cmd(port) {
                out.emit(CliEvent::Info { message: format!("Starting {} gateway on port {}...", desc.display_name, port) });

                if use_native {
                    let backend = clawenv_core::sandbox::native_backend(name);
                    #[cfg(not(target_os = "windows"))]
                    backend.exec(&format!(
                        "nohup {gateway_cmd} > /tmp/clawenv-gateway-{name}.log 2>&1 &"
                    )).await?;
                    #[cfg(target_os = "windows")]
                    backend.exec(&format!(
                        "Start-Process -WindowStyle Hidden -FilePath '{}' -ArgumentList '{}'",
                        desc.cli_binary, desc.gateway_cmd.replace("{port}", &port.to_string())
                    )).await?;
                } else {
                    let backend = detect_backend_for(name)?;
                    // Start ttyd too
                    let ttyd_port = port + 4681;
                    backend.exec(&format!(
                        "nohup ttyd -p {ttyd_port} -W -i 0.0.0.0 sh -c 'cd; exec /bin/sh -l' > /tmp/ttyd.log 2>&1 &"
                    )).await?;
                    backend.exec(&format!(
                        "nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &"
                    )).await?;
                }

                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                out.emit(CliEvent::Complete { message: format!("{} gateway started on port {}", desc.display_name, port) });
            } else {
                out.emit(CliEvent::Complete { message: format!("{} has no gateway (agent runs via terminal)", desc.display_name) });
            }
        }

        other => {
            anyhow::bail!(
                "Unknown install step: '{}'. Valid steps: prereq, create, claw, config, gateway",
                other
            );
        }
    }

    Ok(())
}

/// Classify an error into a structured error code for frontend consumption.
fn classify_error(e: &anyhow::Error) -> String {
    let msg = e.to_string().to_lowercase();
    if msg.contains("not found") && msg.contains("config") {
        "config_not_found".into()
    } else if msg.contains("not found") && msg.contains("instance") {
        "instance_not_found".into()
    } else if msg.contains("not found") && msg.contains("file") {
        "file_not_found".into()
    } else if msg.contains("stalled") || msg.contains("timed out") || msg.contains("timeout") {
        "operation_stalled".into()
    } else if msg.contains("network") || msg.contains("connect") || msg.contains("dns") {
        "network_error".into()
    } else if msg.contains("gateway") && msg.contains("failed") {
        "gateway_failed".into()
    } else if msg.contains("npm install failed") || msg.contains("install failed") {
        "install_failed".into()
    } else if msg.contains("not supported") || msg.contains("not available") {
        "not_supported".into()
    } else if msg.contains("unknown") && (msg.contains("key") || msg.contains("step") || msg.contains("claw")) {
        "invalid_argument".into()
    } else {
        "internal".into()
    }
}
