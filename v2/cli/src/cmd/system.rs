//! `clawcli system <sub>` — host introspection.
//!
//! CLI-DESIGN.md §2.5 + §7.18.
//! - `system info`: OS / arch / memory / disk / sandbox availability
//!   (replaces v1's `system-check`)
//! - `system version`: clawcli version + commit + capabilities
//! - `system state`: launcher state for GUI startup (replaces v1's
//!   `launcher-state`)

use clap::Subcommand;
use clawops_core::wire::{SystemCheckItem, SystemInfo, VersionInfo};

use crate::shared::Ctx;

#[derive(Subcommand)]
pub enum SystemCmd {
    /// Host environment probe (OS, arch, memory, disk, sandbox
    /// availability + checks). Emits `SystemInfo`.
    Info,
    /// clawcli build info + capability list. Emits `VersionInfo`.
    Version,
    /// GUI launcher state (FirstRun / NotInstalled / Ready). Emits
    /// the LaunchState enum directly.
    State,
}

pub async fn run(cmd: SystemCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        SystemCmd::Info    => run_info(ctx).await,
        SystemCmd::Version => run_version(ctx).await,
        SystemCmd::State   => run_state(ctx).await,
    }
}

async fn run_info(ctx: &Ctx) -> anyhow::Result<()> {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let memory_gb = probe_memory_gb().await.unwrap_or(0.0);
    let disk_free_gb = probe_disk_free_gb().await.unwrap_or(0.0);
    let sandbox_backend = if cfg!(target_os = "macos") { "lima" }
        else if cfg!(target_os = "windows") { "wsl2" }
        else { "podman" };
    let sandbox_available = probe_sandbox_available(sandbox_backend).await;

    let mut checks: Vec<SystemCheckItem> = Vec::new();
    checks.push(SystemCheckItem {
        name: "OS".into(),
        ok: !os.is_empty(),
        detail: format!("{os}/{arch}"),
        info_only: false,
    });
    checks.push(SystemCheckItem {
        name: "Memory".into(),
        ok: memory_gb >= 4.0,
        detail: if memory_gb > 0.0 { format!("{memory_gb:.1} GB") } else { "unknown".into() },
        info_only: false,
    });
    checks.push(SystemCheckItem {
        name: "Disk free".into(),
        ok: disk_free_gb >= 10.0,
        detail: if disk_free_gb > 0.0 { format!("{disk_free_gb:.1} GB") } else { "unknown".into() },
        info_only: false,
    });
    checks.push(SystemCheckItem {
        name: "Sandbox backend".into(),
        ok: sandbox_available,
        detail: if sandbox_available {
            format!("{sandbox_backend} ready")
        } else {
            format!("{sandbox_backend} not installed (will be installed on first install)")
        },
        info_only: !sandbox_available,
    });

    let info = SystemInfo {
        os, arch, memory_gb, disk_free_gb,
        sandbox_backend: sandbox_backend.into(),
        sandbox_available,
        checks,
    };
    ctx.emit_pretty(&info, |r| {
        println!("OS         : {}", r.os);
        println!("Arch       : {}", r.arch);
        println!("Memory     : {:.1} GB", r.memory_gb);
        println!("Disk free  : {:.1} GB", r.disk_free_gb);
        println!("Backend    : {} ({})", r.sandbox_backend,
            if r.sandbox_available { "available" } else { "missing" });
        println!("\nChecks:");
        for c in &r.checks {
            let mark = if c.ok { "✓" } else if c.info_only { "ℹ" } else { "✗" };
            println!("  {mark} {}: {}", c.name, c.detail);
        }
    })?;
    Ok(())
}

async fn run_version(ctx: &Ctx) -> anyhow::Result<()> {
    let info = VersionInfo {
        clawcli_version: env!("CARGO_PKG_VERSION").into(),
        // option_env so missing vars don't fail the build; they default
        // to "unknown" which is honest for dev builds.
        commit: option_env!("CLAWCLI_GIT_COMMIT").unwrap_or("unknown").into(),
        build_date: option_env!("CLAWCLI_BUILD_DATE").unwrap_or("unknown").into(),
        capabilities: vec![
            "wire-v2".into(),
            "exec-context".into(),
            "v1-compat:none".into(),
        ],
    };
    ctx.emit_pretty(&info, |i| {
        println!("clawcli {}", i.clawcli_version);
        println!("commit  : {}", i.commit);
        println!("built   : {}", i.build_date);
        println!("caps    : {}", i.capabilities.join(", "));
    })?;
    Ok(())
}

async fn run_state(ctx: &Ctx) -> anyhow::Result<()> {
    let state = clawops_core::launcher::detect_launch_state().await?;
    ctx.emit(&state)?;
    Ok(())
}

// ——— Probe helpers (lifted from v1-compat system-check) ———

async fn probe_memory_gb() -> Option<f64> {
    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"]).output().await.ok()?;
        let bytes: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        return Some(bytes as f64 / 1024.0 / 1024.0 / 1024.0);
    }
    #[cfg(target_os = "linux")]
    {
        let s = tokio::fs::read_to_string("/proc/meminfo").await.ok()?;
        let line = s.lines().find(|l| l.starts_with("MemTotal:"))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        return Some(kb as f64 / 1024.0 / 1024.0);
    }
    #[cfg(target_os = "windows")]
    {
        let out = tokio::process::Command::new("wmic")
            .args(["computersystem", "get", "TotalPhysicalMemory", "/value"])
            .output().await.ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        let line = s.lines().find(|l| l.contains("="))?;
        let bytes: u64 = line.splitn(2, '=').nth(1)?.trim().parse().ok()?;
        return Some(bytes as f64 / 1024.0 / 1024.0 / 1024.0);
    }
    #[allow(unreachable_code)]
    None
}

async fn probe_disk_free_gb() -> Option<f64> {
    let home = clawops_core::paths::clawenv_root().parent()?.to_path_buf();
    #[cfg(unix)]
    {
        let out = tokio::process::Command::new("df")
            .args(["-k", &home.to_string_lossy()])
            .output().await.ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        let line = s.lines().nth(1)?;
        let parts: Vec<&str> = line.split_whitespace().collect();
        let kb: u64 = parts.get(3)?.parse().ok()?;
        return Some(kb as f64 / 1024.0 / 1024.0);
    }
    #[cfg(windows)]
    {
        let drive = home.components().next()?.as_os_str().to_string_lossy().replace(":", "");
        let script = format!(
            "(Get-PSDrive {drive} | Select-Object -ExpandProperty Free) / 1GB"
        );
        let out = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output().await.ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        return s.trim().parse().ok();
    }
    #[allow(unreachable_code)]
    None
}

async fn probe_sandbox_available(backend: &str) -> bool {
    use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};
    let b: Box<dyn SandboxBackend> = match backend {
        "lima"   => Box::new(LimaBackend::new("default")),
        "podman" => Box::new(PodmanBackend::new("default")),
        "wsl2"   => Box::new(WslBackend::new("default")),
        _ => return false,
    };
    b.is_available().await.unwrap_or(false)
}
