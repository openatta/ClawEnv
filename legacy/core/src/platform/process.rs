//! Cross-platform process management utilities.
//!
//! Abstracts platform-specific process operations:
//! - Kill process by name pattern
//! - Check if process is running by name pattern
//! - Open URL in default browser
//! - Get system memory/disk info

use anyhow::Result;

/// Create a tokio Command that won't pop a visible console window on Windows.
/// On non-Windows platforms this is just `tokio::process::Command::new(program)`.
/// For PowerShell: automatically adds `-ExecutionPolicy Bypass` to avoid
/// Restricted policy blocking npm/node scripts.
pub fn silent_cmd(program: &str) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut cmd = tokio::process::Command::new(program);
    // `creation_flags` is provided directly on `tokio::process::Command`
    // (not via std's CommandExt trait) — no `use` import required here
    // and importing std::os::windows::process::CommandExt actually fires
    // unused_imports because nothing else in this function consumes it.
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        if program.to_lowercase().contains("powershell") {
            cmd.args(["-ExecutionPolicy", "Bypass"]);
        }
    }
    cmd
}

/// Kill processes matching a name pattern (force kill).
/// Works in both host context and sandbox context (via SandboxBackend::exec).
///
/// For sandbox: use `backend.exec(&kill_by_name("openclaw gateway"))`
/// For host: use `kill_by_name_host("openclaw gateway").await`
pub fn kill_by_name_cmd(pattern: &str) -> String {
    // Pattern comes from desc.process_names() (TOML descriptor, trusted).
    // Different escaping per platform — unix double-single-quote vs
    // powershell double-quote. Compute each inside its own cfg so the
    // unused branch doesn't fire an unused_variable warning.
    #[cfg(target_os = "windows")]
    {
        let ps_esc = pattern.replace('\'', "''");
        format!(
            "powershell -Command \"Get-Process | Where-Object {{$_.CommandLine -like '*{ps_esc}*'}} | Stop-Process -Force\" 2>$null; exit 0"
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        let esc = pattern.replace('\'', "'\\''");
        format!("pkill -9 -f '{esc}' 2>/dev/null || true")
    }
}

/// Check if a process matching a pattern is running.
/// Returns a shell command that echoes "running" or "stopped".
pub fn check_process_cmd(pattern: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        let ps_esc = pattern.replace('\'', "''");
        format!(
            "powershell -Command \"if (Get-Process | Where-Object {{$_.CommandLine -like '*{ps_esc}*'}}) {{ echo 'running' }} else {{ echo 'stopped' }}\""
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        let esc = pattern.replace('\'', "'\\''");
        format!("pgrep -f '{esc}' > /dev/null 2>&1 && echo running || echo stopped")
    }
}

/// Kill processes matching a name pattern on the HOST (not inside sandbox).
pub async fn kill_by_name_host(pattern: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        silent_cmd("powershell")
            .args(["-Command", &format!(
                "Get-Process | Where-Object {{$_.CommandLine -like '*{pattern}*'}} | Stop-Process -Force"
            )])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        tokio::process::Command::new("sh")
            .args(["-c", &format!("pkill -9 -f '{pattern}' 2>/dev/null || true")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await?;
    }
    Ok(())
}

/// Open a URL in the default browser.
pub async fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        tokio::process::Command::new("open")
            .arg(url)
            .status()
            .await?;
    }
    #[cfg(target_os = "windows")]
    {
        // "start" needs empty title ("") before URL, otherwise URL is treated as window title
        silent_cmd("cmd")
            .args(["/c", "start", "", url])
            .status()
            .await?;
    }
    #[cfg(target_os = "linux")]
    {
        tokio::process::Command::new("xdg-open")
            .arg(url)
            .status()
            .await?;
    }
    Ok(())
}

/// Get total system memory in GB.
pub async fn system_memory_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output().await;
        out.ok().and_then(|o| {
            String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()
        }).unwrap_or(0.0) / 1_073_741_824.0
    }
    #[cfg(target_os = "linux")]
    {
        let out = tokio::fs::read_to_string("/proc/meminfo").await;
        out.ok().and_then(|s| {
            s.lines().find(|l| l.starts_with("MemTotal"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<f64>().ok())
        }).unwrap_or(0.0) / 1_048_576.0
    }
    #[cfg(target_os = "windows")]
    {
        let out = silent_cmd("wmic")
            .args(["ComputerSystem", "get", "TotalPhysicalMemory", "/value"])
            .output().await;
        out.ok().and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines().find(|l| l.contains("="))
                .and_then(|l| l.split('=').nth(1))
                .and_then(|v| v.trim().parse::<f64>().ok())
        }).unwrap_or(0.0) / 1_073_741_824.0
    }
}

/// Get free disk space in GB for the root/system volume.
pub async fn disk_free_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("df")
            .args(["-g", "/"])
            .output().await;
        out.ok().and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines().nth(1)
                .and_then(|l| l.split_whitespace().nth(3))
                .and_then(|v| v.parse::<f64>().ok())
        }).unwrap_or(0.0)
    }
    #[cfg(target_os = "linux")]
    {
        let out = tokio::process::Command::new("df")
            .args(["--output=avail", "-BG", "/"])
            .output().await;
        out.ok().and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines().nth(1)
                .and_then(|l| l.trim().trim_end_matches('G').parse::<f64>().ok())
        }).unwrap_or(0.0)
    }
    #[cfg(target_os = "windows")]
    {
        let out = silent_cmd("wmic")
            .args(["LogicalDisk", "where", "DeviceID='C:'", "get", "FreeSpace", "/value"])
            .output().await;
        out.ok().and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines().find(|l| l.contains("="))
                .and_then(|l| l.split('=').nth(1))
                .and_then(|v| v.trim().parse::<f64>().ok())
        }).unwrap_or(0.0) / 1_073_741_824.0
    }
}
