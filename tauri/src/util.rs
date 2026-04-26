//! Tauri-private platform helpers. Lifted from v1 `clawenv_core::platform::process`
//! since v2 `clawops-core` deliberately stays platform-agnostic for CLI use.

use anyhow::Result;
use clawops_core::proxy::ProxyTriple;

/// Tokio Command that suppresses the console-window flash on Windows.
/// PowerShell additionally gets `-ExecutionPolicy Bypass` so npm/node
/// child scripts aren't blocked. Windows-only — macOS callers gate
/// behind `#[cfg(target_os = "windows")]`.
#[cfg(target_os = "windows")]
pub fn silent_cmd(program: &str) -> tokio::process::Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = tokio::process::Command::new(program);
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    if program.to_lowercase().contains("powershell") {
        cmd.args(["-ExecutionPolicy", "Bypass"]);
    }
    cmd
}

/// Open a URL in the user's default browser.
pub async fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        tokio::process::Command::new("open").arg(url).status().await?;
    }
    #[cfg(target_os = "windows")]
    {
        // `start` needs an empty title arg before the URL, otherwise the URL
        // becomes the window title and never opens.
        silent_cmd("cmd").args(["/c", "start", "", url]).status().await?;
    }
    Ok(())
}

/// Pin `LIMA_HOME` to `~/.clawenv/lima` so every spawned `limactl` uses our
/// private data dir instead of the system default `~/.lima`. Must be called
/// before any code path that may shell out to limactl.
#[cfg(target_os = "macos")]
pub fn init_lima_env() {
    let home = clawops_core::paths::lima_home();
    if let Some(parent) = home.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::env::set_var("LIMA_HOME", &home);
}

/// Inject the resolved proxy triple into this process's env vars so every
/// spawned subprocess (clawcli, native claws) inherits it.
pub fn apply_proxy_env(t: &ProxyTriple) {
    if !t.http.is_empty() {
        std::env::set_var("HTTP_PROXY", &t.http);
        std::env::set_var("http_proxy", &t.http);
    }
    if !t.https.is_empty() {
        std::env::set_var("HTTPS_PROXY", &t.https);
        std::env::set_var("https_proxy", &t.https);
    }
    if !t.no_proxy.is_empty() {
        std::env::set_var("NO_PROXY", &t.no_proxy);
        std::env::set_var("no_proxy", &t.no_proxy);
    }
}

/// Drop proxy env vars when the OS no longer reports a proxy — keeps stale
/// values from sticking around after the user disables their VPN.
pub fn clear_proxy_env() {
    for k in ["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy"] {
        std::env::remove_var(k);
    }
}
