//! WSL2 backend — execute via `wsl -d <distro>`.
//!
//! Port-forwarding on WSL2 is host-side via `netsh interface portproxy`.
//! Only meaningful on Windows; on other hosts most methods return
//! appropriate errors.

#[cfg(target_os = "windows")]
use std::time::Duration;

use async_trait::async_trait;

#[cfg(target_os = "windows")]
use crate::common::{CancellationToken, CommandRunner, CommandSpec};
#[cfg(target_os = "windows")]
use crate::runners::LocalProcessRunner;

use super::{ResourceStats, SandboxBackend};

pub struct WslBackend {
    instance: String,
    #[cfg(target_os = "windows")]
    runner: LocalProcessRunner,
}

impl WslBackend {
    pub fn new(instance: impl Into<String>) -> Self {
        Self {
            instance: instance.into(),
            #[cfg(target_os = "windows")]
            runner: LocalProcessRunner::new(),
        }
    }
}

#[async_trait]
impl SandboxBackend for WslBackend {
    fn name(&self) -> &str { "WSL2" }
    fn instance(&self) -> &str { &self.instance }

    async fn is_available(&self) -> anyhow::Result<bool> {
        #[cfg(not(target_os = "windows"))]
        { Ok(false) }

        #[cfg(target_os = "windows")]
        {
            let spec = CommandSpec::new("wsl", ["-l", "-q", "--running"])
                .with_timeout(Duration::from_secs(5));
            let res = self.runner.exec(spec, CancellationToken::new()).await?;
            if !res.success() { return Ok(false); }
            Ok(res.stdout.lines().any(|l| l.trim() == self.instance))
        }
    }

    async fn start(&self) -> anyhow::Result<()> {
        #[cfg(not(target_os = "windows"))]
        { anyhow::bail!("WSL2 is only available on Windows"); }

        #[cfg(target_os = "windows")]
        {
            // Starting a WSL distro is implicit on first exec; issue a no-op.
            let spec = CommandSpec::new("wsl", ["-d", &self.instance, "--", "true"])
                .with_timeout(Duration::from_secs(60));
            let res = self.runner.exec(spec, CancellationToken::new()).await?;
            if !res.success() {
                anyhow::bail!("wsl start {} failed: {}", self.instance, res.stderr);
            }
            Ok(())
        }
    }

    async fn stop(&self) -> anyhow::Result<()> {
        #[cfg(not(target_os = "windows"))]
        { anyhow::bail!("WSL2 is only available on Windows"); }

        #[cfg(target_os = "windows")]
        {
            let spec = CommandSpec::new("wsl", ["--terminate", &self.instance])
                .with_timeout(Duration::from_secs(30));
            let res = self.runner.exec(spec, CancellationToken::new()).await?;
            if !res.success() {
                anyhow::bail!("wsl terminate {} failed: {}", self.instance, res.stderr);
            }
            Ok(())
        }
    }

    async fn exec(&self, cmd: &str) -> anyhow::Result<String> {
        #[cfg(not(target_os = "windows"))]
        { let _ = cmd; anyhow::bail!("WSL2 is only available on Windows"); }

        #[cfg(target_os = "windows")]
        {
            let spec = CommandSpec::new("wsl",
                ["-d", &self.instance, "--", "sh", "-c", cmd])
                .with_timeout(Duration::from_secs(5 * 60));
            let res = self.runner.exec(spec, CancellationToken::new()).await?;
            if !res.success() {
                anyhow::bail!("wsl exec failed (exit {}): {}", res.exit_code, res.stderr);
            }
            Ok(res.stdout)
        }
    }

    async fn stats(&self) -> anyhow::Result<ResourceStats> {
        // WSL2 exposes `wmic`/PowerShell process stats but not per-distro
        // CPU/memory uniformly. Stage D can query via PowerShell cmdlets.
        Ok(ResourceStats::default())
    }

    async fn edit_port_forwards(&self, _forwards: &[(u16, u16)]) -> anyhow::Result<()> {
        #[cfg(not(target_os = "windows"))]
        { anyhow::bail!("WSL2 port-forward edit requires Windows"); }

        #[cfg(target_os = "windows")]
        {
            // Strategy: clear ALL existing v4tov4 rules, then add each new one.
            // This is per v1's edit_port_forwards contract (set-all semantics).
            let clear = CommandSpec::new("netsh",
                ["interface", "portproxy", "reset"])
                .with_timeout(Duration::from_secs(10));
            let _ = self.runner.exec(clear, CancellationToken::new()).await;

            for (host, guest) in _forwards {
                // Look up WSL IP lazily via `wsl hostname -I` — first field is the distro IP.
                let ip_spec = CommandSpec::new("wsl",
                    ["-d", &self.instance, "--", "hostname", "-I"])
                    .with_timeout(Duration::from_secs(5));
                let ip_res = self.runner.exec(ip_spec, CancellationToken::new()).await?;
                if !ip_res.success() {
                    anyhow::bail!("cannot resolve WSL IP: {}", ip_res.stderr);
                }
                let ip = ip_res.stdout.split_whitespace().next()
                    .ok_or_else(|| anyhow::anyhow!("empty WSL IP"))?.to_string();

                let args = [
                    "interface", "portproxy", "add", "v4tov4",
                    &format!("listenport={host}"),
                    "listenaddress=0.0.0.0",
                    &format!("connectport={guest}"),
                    &format!("connectaddress={ip}"),
                ];
                let add = CommandSpec::new("netsh", args)
                    .with_timeout(Duration::from_secs(10));
                let res = self.runner.exec(add, CancellationToken::new()).await?;
                if !res.success() {
                    anyhow::bail!("netsh portproxy add failed: {}", res.stderr);
                }
            }
            Ok(())
        }
    }

    fn supports_rename(&self) -> bool { true }
    fn supports_resource_edit(&self) -> bool { true }
    fn supports_port_edit(&self) -> bool { true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_and_name() {
        let b = WslBackend::new("demo");
        assert_eq!(b.name(), "WSL2");
        assert_eq!(b.instance(), "demo");
    }
}
