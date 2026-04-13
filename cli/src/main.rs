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
    /// Set a configuration value
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

    let result = run(cli.command, &out).await;

    match result {
        Ok(()) => {
            std::process::exit(0);
        }
        Err(e) => {
            out.emit(CliEvent::Error { message: e.to_string() });
            std::process::exit(1);
        }
    }
}

async fn run(command: Commands, out: &Output) -> Result<()> {
    use clawenv_core::api::*;
    use clawenv_core::config::{ConfigManager, UserMode};

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

            let actual_port = if port == 0 { desc.default_port } else { port };
            let use_native = mode == "native";

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
                            stage: format!("{:?}", progress.stage),
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
                            sandbox_type: format!("{:?}", inst.sandbox_type),
                            health: format!("{:?}", health),
                            gateway_port: inst.gateway.gateway_port,
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
                sandbox_type: format!("{:?}", inst.sandbox_type),
                health: format!("{:?}", health),
                gateway_port: inst.gateway.gateway_port,
                ttyd_port: inst.gateway.ttyd_port,
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
                format!("{} logs --lines 100", desc.cli_binary)
            };
            let output = backend.exec(&cmd).await?;
            // Logs are raw text, not JSON events
            print!("{output}");
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
            let npm_registry = config.config().clawenv.mirrors.npm_registry_url().to_string();
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let claw_reg = clawenv_core::claw::ClawRegistry::load();
            let desc = claw_reg.get(&inst.claw_type);

            match clawenv_core::update::checker::check_latest_version(&inst.version, &npm_registry, &desc.npm_package).await {
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
        Commands::Export { name, output } => {
            let name = resolve_name(name);
            let config = ConfigManager::load()?;
            let inst = clawenv_core::manager::instance::get_instance(&config, &name)?;
            let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;

            std::fs::create_dir_all(&output)?;
            let claw_reg = clawenv_core::claw::ClawRegistry::load();
            let desc = claw_reg.get(&inst.claw_type);

            out.emit(CliEvent::Info { message: format!("Exporting '{name}'...") });

            let version = backend.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await.unwrap_or_default();
            out.emit(CliEvent::Info { message: format!("{}: {}", desc.display_name, version.trim()) });

            backend.stop().await.ok();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let status = tokio::process::Command::new("bash")
                .args(["scripts/package-alpine.sh", &name, &output])
                .status().await?;

            backend.start().await.ok();

            if status.success() {
                out.emit(CliEvent::Complete { message: format!("Exported to {output}/") });
            } else {
                anyhow::bail!("Export failed");
            }
        }

        // ====== Import ======
        Commands::Import { file, name } => {
            let path = std::path::PathBuf::from(&file);
            if !path.exists() {
                anyhow::bail!("File not found: {}", path.display());
            }
            out.emit(CliEvent::Info { message: format!("Importing '{name}' from {file}...") });
            let backend = clawenv_core::sandbox::detect_backend()?;
            backend.import_image(&path).await?;
            out.emit(CliEvent::Complete { message: format!("Imported. Run 'clawenv start {name}' to start.") });
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
                npm_package: d.npm_package.clone(),
                default_port: d.default_port,
                supports_mcp: d.supports_mcp,
                supports_browser: d.supports_browser,
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

            #[cfg(target_os = "windows")]
            {
                let proxy = clawenv_core::platform::process::silent_cmd("reg")
                    .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings", "/v", "ProxyEnable"])
                    .output().await;
                let has_proxy = proxy.map(|o| String::from_utf8_lossy(&o.stdout).contains("0x1")).unwrap_or(false);
                checks.push(CheckItem { name: "Proxy".into(), ok: true, detail: (if has_proxy { "Detected" } else { "None" }).into(), info_only: false });
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
            clawenv_core::manager::instance::stop_instance(&inst).await.ok();

            let new_sandbox_id = if backend.supports_rename() {
                backend.rename(&new_name).await?
            } else {
                format!("{:?}-{}", inst.sandbox_type, new_name).to_lowercase()
            };

            if let Some(entry) = config.config_mut().instances.iter_mut().find(|i| i.name == old_name) {
                entry.name = new_name.clone();
                entry.sandbox_id = new_sandbox_id;
            }
            config.save()?;

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
                clawenv_core::manager::instance::stop_instance(inst).await.ok();
                backend.edit_resources(cpus, memory, disk).await?;
                out.emit(CliEvent::Info { message: "Resources updated".into() });
            }

            // Edit ports if any port flags provided
            if gateway_port.is_some() || ttyd_port.is_some() {
                let gp = gateway_port.unwrap_or(inst.gateway.gateway_port);
                let tp = ttyd_port.unwrap_or(inst.gateway.ttyd_port);

                let backend = clawenv_core::manager::instance::backend_for_instance(inst)?;
                if backend.supports_port_edit() {
                    clawenv_core::manager::instance::stop_instance(inst).await.ok();
                    backend.edit_port_forwards(&[(gp, gp), (tp, tp)]).await?;
                }

                if let Some(entry) = config.config_mut().instances.iter_mut().find(|i| i.name == name) {
                    entry.gateway.gateway_port = gp;
                    entry.gateway.ttyd_port = tp;
                }
                config.save()?;
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
                        let output = tokio::process::Command::new("limactl")
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
                                    });
                                }
                            }
                        }
                    }

                    #[cfg(target_os = "windows")]
                    {
                        let output = clawenv_core::platform::process::silent_cmd("wsl")
                            .args(["--list", "--verbose"])
                            .output().await;
                        if let Ok(o) = output {
                            for line in String::from_utf8_lossy(&o.stdout).lines().skip(1) {
                                let p: Vec<&str> = line.split_whitespace().collect();
                                if p.len() >= 3 {
                                    let name = p[0].trim_start_matches('*').trim();
                                    vms.push(SandboxVmInfo {
                                        name: name.into(), status: p[1].into(),
                                        cpus: "-".into(), memory: "-".into(),
                                        disk: "-".into(), dir_size: "-".into(),
                                        managed: name.starts_with("ClawEnv"),
                                    });
                                }
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
                            std::process::Command::new("limactl")
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
                        mirror_preset: c.clawenv.mirrors.preset.clone(),
                        bridge_enabled: c.clawenv.bridge.enabled,
                        bridge_port: c.clawenv.bridge.port,
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
    use clawenv_core::sandbox::{detect_backend, SandboxBackend, SandboxOpts, SandboxType, InstallMode};
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
                let backend = detect_backend()?;
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
                let backend = detect_backend()?;
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
                let backend = detect_backend()?;
                let already = backend.exec(&format!("which {} 2>/dev/null", desc.cli_binary)).await
                    .map(|o| !o.trim().is_empty()).unwrap_or(false);
                if already {
                    let ver = backend.exec(&desc.version_check_cmd()).await.unwrap_or_default();
                    out.emit(CliEvent::Complete { message: format!("{} {} already installed", desc.display_name, ver.trim()) });
                } else {
                    let cmd = desc.npm_install_verbose_cmd(version);
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
                        let b = detect_backend()?;
                        b.exec(&format!("{cmd} 2>/dev/null || true")).await?;
                    }
                }
            }

            // Get version
            let claw_version = if use_native {
                let b = clawenv_core::sandbox::native_backend(name);
                b.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await.unwrap_or_default()
            } else {
                let b = detect_backend()?;
                b.exec(&format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd())).await.unwrap_or_default()
            };

            let sandbox_type = if use_native { SandboxType::Native } else { SandboxType::from_os() };
            let sandbox_id = if use_native { format!("native-{name}") } else { format!("clawenv-{name}") };
            let ttyd_port = if use_native { 0 } else { port + 4681 };

            config.config_mut().instances.retain(|i| i.name != name);
            config.config_mut().instances.push(InstanceConfig {
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
                    webchat_enabled: true,
                    channels: Default::default(),
                },
                resources: ResourceConfig::default(),
                browser: Default::default(),
                cached_latest_version: String::new(),
                cached_version_check_at: String::new(),
            });
            config.save()?;
            out.emit(CliEvent::Complete { message: format!("Instance '{}' config saved (port {})", name, port) });
        }

        // ---- Step: gateway ----
        "gateway" => {
            let gateway_cmd = desc.gateway_start_cmd(port);
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
                let backend = detect_backend()?;
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
