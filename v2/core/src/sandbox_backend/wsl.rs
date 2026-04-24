//! WSL2 backend — execute via `wsl -d <distro>`.
//!
//! Port-forwarding on WSL2 is host-side via `netsh interface portproxy`.
//! Only meaningful on Windows; on other hosts most methods return
//! appropriate errors.

#[cfg(target_os = "windows")]
use std::time::Duration;

use async_trait::async_trait;

#[cfg(target_os = "windows")]
use crate::common::{CancellationToken, CommandRunner, CommandSpec, ProgressSink};
#[cfg(target_os = "windows")]
use crate::download_ops::{CatalogBackedDownloadOps, DownloadOps};
#[cfg(target_os = "windows")]
use crate::paths::v2_config_dir;
use crate::provisioning::CreateOpts;
#[cfg(target_os = "windows")]
use crate::provisioning::render_wsl_provision_script;
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

    async fn is_present(&self) -> anyhow::Result<bool> {
        #[cfg(not(target_os = "windows"))]
        { Ok(false) }

        #[cfg(target_os = "windows")]
        {
            // `wsl -l -q` lists every registered distro, running or not.
            let spec = CommandSpec::new("wsl", ["-l", "-q"])
                .with_timeout(Duration::from_secs(5));
            let res = self.runner.exec(spec, CancellationToken::new()).await?;
            if !res.success() { return Ok(false); }
            Ok(res.stdout.lines().any(|l| l.trim() == self.instance))
        }
    }

    async fn create(&self, opts: &CreateOpts) -> anyhow::Result<()> {
        #[cfg(not(target_os = "windows"))]
        { let _ = opts; anyhow::bail!("WslBackend::create requires Windows"); }

        #[cfg(target_os = "windows")]
        {
            // Idempotency: already registered = done.
            if self.is_present().await.unwrap_or(false) {
                return Ok(());
            }

            // 1) Fetch Alpine minirootfs via DownloadOps. Cached after first run.
            let downloader = CatalogBackedDownloadOps::with_defaults();
            let rootfs = downloader
                .fetch("alpine-rootfs", None, ProgressSink::noop(), CancellationToken::new())
                .await
                .map_err(|e| anyhow::anyhow!("fetch alpine-rootfs: {e}"))?;

            // 2) Prepare distro-local dir under our v2 config root.
            let distro_dir = v2_config_dir().join("wsl").join(&self.instance);
            tokio::fs::create_dir_all(&distro_dir).await
                .map_err(|e| anyhow::anyhow!(
                    "create WSL distro dir {}: {e}", distro_dir.display()
                ))?;

            // 3) wsl --import <distro> <dir> <rootfs> --version 2
            let import_spec = CommandSpec::new("wsl", [
                "--import",
                self.instance.as_str(),
                distro_dir.to_str().ok_or_else(||
                    anyhow::anyhow!("non-UTF8 WSL dir: {}", distro_dir.display())
                )?,
                rootfs.to_str().ok_or_else(||
                    anyhow::anyhow!("non-UTF8 rootfs: {}", rootfs.display())
                )?,
                "--version", "2",
            ]).with_timeout(Duration::from_secs(5 * 60));
            let res = self.runner.exec(import_spec, CancellationToken::new()).await?;
            if !res.success() {
                anyhow::bail!(
                    "wsl --import {}: exit {}\nstderr:\n{}",
                    self.instance, res.exit_code, res.stderr
                );
            }

            // 4) Provision via inline script. Unlike Lima/Podman there's no
            //    separate "first-boot cloud-init" hook; we just exec the
            //    script synchronously and let WSL keep us blocked.
            //    The heredoc marker is non-occurring ASCII and the body is
            //    our trusted render — no shell-injection risk.
            let script = render_wsl_provision_script(opts);
            let inline = format!(
                "cat > /tmp/clawenv-provision.sh << 'CLAWOPS_WSL_EOF'\n\
                 {script}\n\
                 CLAWOPS_WSL_EOF\n\
                 chmod +x /tmp/clawenv-provision.sh\n\
                 /bin/sh /tmp/clawenv-provision.sh"
            );
            let prov_spec = CommandSpec::new("wsl", [
                "-d", self.instance.as_str(), "--",
                "sh", "-c", inline.as_str(),
            ]).with_timeout(Duration::from_secs(20 * 60));
            let res = self.runner.exec(prov_spec, CancellationToken::new()).await?;
            if !res.success() {
                anyhow::bail!(
                    "wsl provision in {}: exit {}\nstderr:\n{}",
                    self.instance, res.exit_code, res.stderr
                );
            }
            Ok(())
        }
    }

    async fn destroy(&self) -> anyhow::Result<()> {
        #[cfg(not(target_os = "windows"))]
        { Ok(()) }

        #[cfg(target_os = "windows")]
        {
            // Idempotent — missing = done.
            if !self.is_present().await.unwrap_or(false) {
                return Ok(());
            }
            let spec = CommandSpec::new("wsl", ["--unregister", &self.instance])
                .with_timeout(Duration::from_secs(2 * 60));
            let res = self.runner.exec(spec, CancellationToken::new()).await?;
            if !res.success() {
                anyhow::bail!(
                    "wsl --unregister {}: {}", self.instance, res.stderr
                );
            }
            // Clean up our distro dir (best-effort).
            let distro_dir = v2_config_dir().join("wsl").join(&self.instance);
            let _ = tokio::fs::remove_dir_all(&distro_dir).await;
            Ok(())
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

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn create_fails_cleanly_on_non_windows() {
        let b = WslBackend::new("demo");
        let err = b.create(&crate::provisioning::CreateOpts::minimal("demo", "openclaw"))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("requires Windows"));
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn destroy_is_noop_on_non_windows() {
        // Non-Windows: is_present returns false, destroy short-circuits Ok.
        let b = WslBackend::new("demo");
        b.destroy().await.unwrap();
    }
}
