use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "clawenv", version, about = "OpenClaw sandbox installer & manager")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install OpenClaw in a sandbox
    Install {
        #[arg(long, default_value = "sandbox")]
        mode: String,
        #[arg(long, default_value = "latest")]
        version: String,
        #[arg(long, default_value = "default")]
        name: String,
        /// Path to local image file for offline install
        #[arg(long)]
        image: Option<String>,
        /// Install browser (Chromium + noVNC)
        #[arg(long)]
        browser: bool,
        /// Gateway port
        #[arg(long, default_value = "3000")]
        port: u16,
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
    Status {
        name: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show instance logs
    Logs {
        name: Option<String>,
        #[arg(short, long)]
        follow: bool,
    },
    /// Sandbox operations (developer mode)
    Sandbox {
        #[command(subcommand)]
        command: SandboxCommands,
    },
    /// Snapshot management
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommands,
    },
    /// Upgrade OpenClaw to latest or specific version
    Upgrade {
        name: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },
    /// Rollback to a previous snapshot
    Rollback {
        name: Option<String>,
        #[arg(long)]
        to: String,
    },
    /// Check for available updates
    UpdateCheck {
        name: Option<String>,
    },
    /// Export an instance as a distributable package
    Export {
        /// Instance name
        name: Option<String>,
        /// Output directory
        #[arg(long, default_value = "./packages")]
        output: String,
    },
    /// Import an instance from a package file
    Import {
        /// Path to package file (.tar.gz / .tar / .qcow2)
        file: String,
        /// Instance name
        #[arg(long, default_value = "default")]
        name: String,
    },
    /// Diagnose current environment
    Doctor,
}

#[derive(Subcommand)]
enum SandboxCommands {
    /// Execute a command in sandbox
    Exec { cmd: String, name: Option<String> },
}

#[derive(Subcommand)]
enum SnapshotCommands {
    Create { tag: String, name: Option<String> },
    Restore { tag: String, name: Option<String> },
    List { name: Option<String> },
}

fn resolve_name(name: Option<String>) -> String {
    name.unwrap_or_else(|| "default".into())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Install { mode, version, name, image, browser, port } => {
            use clawenv_core::config::{ConfigManager, UserMode};
            use clawenv_core::manager::install::{self, InstallOptions};
            use clawenv_core::sandbox::{InstallMode, ImageSource};

            let install_mode = if let Some(ref img_path) = image {
                InstallMode::PrebuiltImage {
                    source: ImageSource::LocalFile { path: img_path.into() },
                }
            } else {
                InstallMode::OnlineBuild
            };

            let opts = InstallOptions {
                instance_name: name,
                claw_version: version,
                install_mode,
                install_browser: browser,
                api_key: None,
                use_native: mode == "native",
                gateway_port: port,
            };

            let mut config = ConfigManager::load()
                .or_else(|_| ConfigManager::create_default(UserMode::General))?;

            let (tx, mut rx) = tokio::sync::mpsc::channel::<install::InstallProgress>(32);

            // Print progress in terminal
            let print_task = tokio::spawn(async move {
                while let Some(progress) = rx.recv().await {
                    let bar = "█".repeat((progress.percent as usize) / 5);
                    let empty = "░".repeat(20 - (progress.percent as usize) / 5);
                    println!("\r  [{bar}{empty}] {}% {}", progress.percent, progress.message);
                }
            });

            println!("Installing OpenClaw...\n");
            install::install(opts, &mut config, tx).await?;
            print_task.await?;
            println!("\nDone!");
        }

        Commands::Uninstall { name } => {
            let mut config = clawenv_core::config::ConfigManager::load()?;
            clawenv_core::manager::instance::remove_instance(&mut config, &name).await?;
            println!("Instance '{name}' removed.");
        }

        Commands::List => {
            match clawenv_core::config::ConfigManager::load() {
                Ok(config) => {
                    if config.instances().is_empty() {
                        println!("No instances found.");
                    } else {
                        println!("{:<15} {:<12} {:<10} {:<15} {:>5}", "NAME", "TYPE", "VERSION", "SANDBOX", "PORT");
                        println!("{}", "─".repeat(60));
                        for inst in config.instances() {
                            let health = clawenv_core::manager::instance::instance_health(inst).await;
                            let status = match health {
                                clawenv_core::monitor::InstanceHealth::Running => "● running",
                                clawenv_core::monitor::InstanceHealth::Stopped => "○ stopped",
                                clawenv_core::monitor::InstanceHealth::Unreachable => "✕ unreachable",
                            };
                            println!(
                                "{:<15} {:<12} {:<10} {:<15} {:>5}",
                                inst.name, inst.claw_type, inst.version,
                                format!("{:?}", inst.sandbox_type),
                                inst.openclaw.gateway_port,
                            );
                            println!("  {status}");
                        }
                    }
                }
                Err(_) => println!("No configuration found. Run 'clawenv install' first."),
            }
        }

        Commands::Start { name } => {
            let name = resolve_name(name);
            let config = clawenv_core::config::ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            clawenv_core::manager::instance::start_instance(inst).await?;
            println!("Instance '{name}' started.");
        }

        Commands::Stop { name } => {
            let name = resolve_name(name);
            let config = clawenv_core::config::ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            clawenv_core::manager::instance::stop_instance(inst).await?;
            println!("Instance '{name}' stopped.");
        }

        Commands::Restart { name } => {
            let name = resolve_name(name);
            let config = clawenv_core::config::ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            clawenv_core::manager::instance::restart_instance(inst).await?;
            println!("Instance '{name}' restarted.");
        }

        Commands::Status { name, json } => {
            let name = resolve_name(name);
            let config = clawenv_core::config::ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let health = clawenv_core::manager::instance::instance_health(inst).await;
            if json {
                println!("{}", serde_json::json!({
                    "name": inst.name,
                    "version": inst.version,
                    "sandbox_type": format!("{:?}", inst.sandbox_type),
                    "health": format!("{:?}", health),
                    "gateway_port": inst.openclaw.gateway_port,
                }));
            } else {
                println!("Instance: {}", inst.name);
                println!("Version:  {}", inst.version);
                println!("Sandbox:  {:?}", inst.sandbox_type);
                println!("Health:   {:?}", health);
                println!("Gateway:  127.0.0.1:{}", inst.openclaw.gateway_port);
            }
        }

        Commands::Logs { name, follow } => {
            let name = resolve_name(name);
            let config = clawenv_core::config::ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
            let cmd = if follow { "openclaw logs -f" } else { "openclaw logs --lines 100" };
            let output = backend.exec(cmd).await?;
            print!("{output}");
        }

        Commands::Sandbox { command } => match command {
            SandboxCommands::Exec { cmd, name } => {
                let name = resolve_name(name);
                let config = clawenv_core::config::ConfigManager::load()?;
                let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
                let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
                let output = backend.exec(&cmd).await?;
                print!("{output}");
            }
        },

        Commands::Snapshot { command } => match command {
            SnapshotCommands::Create { tag, name } => {
                let name = resolve_name(name);
                let config = clawenv_core::config::ConfigManager::load()?;
                let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
                let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
                backend.snapshot_create(&tag).await?;
                println!("Snapshot '{tag}' created for instance '{name}'.");
            }
            SnapshotCommands::Restore { tag, name } => {
                let name = resolve_name(name);
                let config = clawenv_core::config::ConfigManager::load()?;
                let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
                let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
                backend.snapshot_restore(&tag).await?;
                println!("Snapshot '{tag}' restored for instance '{name}'.");
            }
            SnapshotCommands::List { name } => {
                let name = resolve_name(name);
                let config = clawenv_core::config::ConfigManager::load()?;
                let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
                let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
                let snaps = backend.snapshot_list().await?;
                if snaps.is_empty() {
                    println!("No snapshots for instance '{name}'.");
                } else {
                    for s in snaps {
                        println!("  {} ({})", s.tag, s.created_at);
                    }
                }
            }
        },

        Commands::Upgrade { name, version } => {
            let name = resolve_name(name);
            let mut config = clawenv_core::config::ConfigManager::load()?;
            println!("Upgrading instance '{name}'...");
            let new_ver = clawenv_core::manager::upgrade::upgrade(
                &mut config,
                &name,
                version.as_deref(),
            ).await?;
            println!("Upgraded to {new_ver}");
        }

        Commands::Rollback { name, to } => {
            let name = resolve_name(name);
            let config = clawenv_core::config::ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            clawenv_core::manager::upgrade::rollback(inst, &to).await?;
            println!("Rolled back instance '{name}' to snapshot '{to}'.");
        }

        Commands::UpdateCheck { name } => {
            let name = resolve_name(name);
            let config = clawenv_core::config::ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            println!("Checking for updates...");
            match clawenv_core::update::checker::check_latest_version(&inst.version).await {
                Ok(info) => {
                    if info.has_upgrade() {
                        println!("Update available: {} → {}", info.current, info.latest);
                        if info.is_security_release {
                            println!("  ⚠ This is a SECURITY release!");
                        }
                        println!("\nChangelog:\n{}", info.changelog);
                    } else {
                        println!("Already up to date (v{}).", info.current);
                    }
                }
                Err(e) => println!("Failed to check updates: {e}"),
            }
        }

        Commands::Export { name, output } => {
            let name = resolve_name(name);
            let config = clawenv_core::config::ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;

            std::fs::create_dir_all(&output)?;
            let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            let platform = std::env::consts::OS;
            let arch = std::env::consts::ARCH;
            let filename = format!("clawenv-{name}-{timestamp}-{platform}-{arch}.tar.gz");
            let outpath = std::path::PathBuf::from(&output).join(&filename);

            println!("Exporting instance '{name}'...");

            // Get version info
            let version = backend.exec("openclaw --version 2>/dev/null || echo unknown").await.unwrap_or_default();
            println!("  OpenClaw: {}", version.trim());

            // Stop VM for clean export
            println!("  Stopping instance...");
            backend.stop().await.ok();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            // Use the platform package script
            println!("  Packaging (this may take a minute)...");
            let status = tokio::process::Command::new("bash")
                .args(["scripts/package-alpine.sh", &name, &output])
                .status()
                .await?;

            if !status.success() {
                println!("  Package script failed. Trying direct export...");
                // Fallback: just note the location
                println!("  Lima instance at: ~/.lima/clawenv-{name}/");
            }

            // Restart
            println!("  Restarting instance...");
            backend.start().await.ok();

            println!("\nExport complete: {}", outpath.display());
        }

        Commands::Import { file, name } => {
            let path = std::path::PathBuf::from(&file);
            if !path.exists() {
                anyhow::bail!("File not found: {}", path.display());
            }

            println!("Importing instance '{name}' from {}...", path.display());

            let backend = clawenv_core::sandbox::detect_backend()?;
            backend.import_image(&path).await?;

            println!("Import complete. Run 'clawenv start {name}' to start.");
        }

        Commands::Doctor => {
            let platform = clawenv_core::platform::detect_platform()?;
            println!("ClawEnv Doctor");
            println!("──────────────");
            println!("OS:       {:?}", platform.os);
            println!("Arch:     {:?}", platform.arch);

            let backend = clawenv_core::sandbox::detect_backend();
            match backend {
                Ok(b) => {
                    let available = b.is_available().await?;
                    println!("Backend:  {} (available: {})", b.name(), available);
                }
                Err(e) => println!("Backend:  not available ({e})"),
            }

            match clawenv_core::config::ConfigManager::load() {
                Ok(config) => {
                    println!("Config:   loaded ({} instances)", config.instances().len());
                    for inst in config.instances() {
                        let health = clawenv_core::manager::instance::instance_health(inst).await;
                        println!("  - {} v{} ({:?}) → {:?}", inst.name, inst.version, inst.sandbox_type, health);
                    }
                }
                Err(_) => println!("Config:   not found (~/.clawenv/config.toml)"),
            }
        }
    }

    Ok(())
}
