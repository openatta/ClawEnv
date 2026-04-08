use anyhow::Result;

/// Detect the host machine's LAN IP address.
///
/// Lima VZ gateway (host.lima.internal / 192.168.5.2) only handles DNS/NAT —
/// it does NOT forward to host user-space ports. We need the real LAN IP
/// so sandbox VMs can reach Bridge Server, ttyd, etc.
///
/// Detection strategy (ordered by reliability):
///   1. Python socket trick (cross-platform, most reliable)
///   2. macOS: `ipconfig getifaddr en0`
///   3. Linux: `hostname -I | awk '{print $1}'`
///   4. Windows/WSL: `powershell (Get-NetIPAddress ...).IPAddress`
pub async fn detect_host_ip() -> Result<String> {
    // Strategy 1: Python UDP socket (works on all platforms)
    let out = tokio::process::Command::new("python3")
        .args(["-c", "import socket; s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM); s.connect(('8.8.8.8',80)); print(s.getsockname()[0]); s.close()"])
        .output()
        .await;

    if let Ok(out) = out {
        let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if is_valid_lan_ip(&ip) {
            return Ok(ip);
        }
    }

    // Strategy 2: Platform-specific fallbacks
    #[cfg(target_os = "macos")]
    {
        let out = tokio::process::Command::new("ipconfig")
            .args(["getifaddr", "en0"])
            .output()
            .await;
        if let Ok(out) = out {
            let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if is_valid_lan_ip(&ip) {
                return Ok(ip);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let out = tokio::process::Command::new("hostname")
            .arg("-I")
            .output()
            .await;
        if let Ok(out) = out {
            let ip = String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if is_valid_lan_ip(&ip) {
                return Ok(ip);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let out = tokio::process::Command::new("powershell")
            .args(["-Command", "(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -notlike '*Loopback*' -and $_.PrefixOrigin -eq 'Dhcp' } | Select-Object -First 1).IPAddress"])
            .output()
            .await;
        if let Ok(out) = out {
            let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if is_valid_lan_ip(&ip) {
                return Ok(ip);
            }
        }
    }

    anyhow::bail!("Could not detect host LAN IP")
}

/// Check if an IP string looks like a valid LAN IPv4 address
fn is_valid_lan_ip(ip: &str) -> bool {
    if ip.is_empty() || !ip.contains('.') {
        return false;
    }
    // Must parse as valid IPv4
    ip.parse::<std::net::Ipv4Addr>().is_ok()
}

/// Read the host IP currently configured inside a sandbox VM.
/// Returns None if not set.
pub async fn read_sandbox_host_ip(backend: &dyn crate::sandbox::SandboxBackend) -> Option<String> {
    let out = backend
        .exec("grep -oP 'CLAWENV_HOST_IP=\\K.*' /etc/profile.d/clawenv-host.sh 2>/dev/null || echo ''")
        .await
        .unwrap_or_default();
    let ip = out.trim().to_string();
    if ip.is_empty() { None } else { Some(ip) }
}

/// Update the host IP inside a sandbox VM if it has changed.
/// Returns Ok(true) if updated, Ok(false) if unchanged.
pub async fn sync_host_ip(backend: &dyn crate::sandbox::SandboxBackend) -> Result<bool> {
    let current_ip = detect_host_ip().await?;
    let stored_ip = read_sandbox_host_ip(backend).await;

    if stored_ip.as_deref() == Some(&current_ip) {
        tracing::debug!("Host IP unchanged: {current_ip}");
        return Ok(false);
    }

    tracing::info!(
        "Host IP changed: {} → {current_ip}",
        stored_ip.as_deref().unwrap_or("(unset)")
    );

    // Update the env file
    backend
        .exec(&format!(
            "echo 'CLAWENV_HOST_IP={current_ip}' | sudo tee /etc/profile.d/clawenv-host.sh > /dev/null"
        ))
        .await?;

    Ok(true)
}
