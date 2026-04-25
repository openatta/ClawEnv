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

    async fn ensure_prerequisites(&self) -> anyhow::Result<()> {
        #[cfg(not(target_os = "windows"))]
        { anyhow::bail!("WSL2 is the Windows sandbox backend; on this host use Lima (macOS) or Podman (Linux)"); }

        #[cfg(target_os = "windows")]
        {
            // Lifted from v1 sandbox/wsl.rs:180-323. Three-stage check:
            // (1) Hardware: Hyper-V feature detectable AND if we're in a
            //     VM, nested-virt is on. (2) WSL features: WSL +
            //     VirtualMachinePlatform are enabled in dism. (3) If both
            //     enabled, set WSL2 default + update kernel. Otherwise
            //     prompt UAC and run dism /enable-feature for both.
            //
            // dism + Get-WmiObject + Get-CimInstance produce no window
            // flash; the UAC PowerShell with /enable-feature INTENTIONALLY
            // shows a terminal window so the user sees the elevation
            // prompt and progress.

            // Stage 1: hardware virtualization probe.
            let hyperv_check = LocalProcessRunner::new().exec(
                CommandSpec::new("dism", [
                    "/online", "/get-featureinfo",
                    "/featurename:Microsoft-Hyper-V", "/English"
                ]).with_timeout(Duration::from_secs(15)),
                CancellationToken::new(),
            ).await;
            let _hyperv_available = hyperv_check.as_ref()
                .map(|res| res.stdout.contains("State :") && !res.stdout.contains("not recognized"))
                .unwrap_or(false);

            // Are we inside a VM that lacks nested virtualization?
            let vm_check = LocalProcessRunner::new().exec(
                CommandSpec::new("powershell", [
                    "-WindowStyle", "Hidden", "-Command",
                    "(Get-WmiObject Win32_ComputerSystem).Model"
                ]).with_timeout(Duration::from_secs(10)),
                CancellationToken::new(),
            ).await;
            let is_virtual = vm_check.as_ref()
                .map(|res| {
                    let m = res.stdout.to_lowercase();
                    m.contains("virtual") || m.contains("vmware") || m.contains("qemu")
                        || m.contains("utm") || m.contains("parallels")
                })
                .unwrap_or(false);

            if is_virtual {
                let nested = LocalProcessRunner::new().exec(
                    CommandSpec::new("powershell", [
                        "-WindowStyle", "Hidden", "-Command",
                        "(Get-CimInstance Win32_Processor).VirtualizationFirmwareEnabled"
                    ]).with_timeout(Duration::from_secs(10)),
                    CancellationToken::new(),
                ).await;
                let nested_ok = nested.as_ref()
                    .map(|r| r.stdout.trim().contains("True"))
                    .unwrap_or(false);
                if !nested_ok {
                    anyhow::bail!(
                        "WSL2 requires hardware virtualization, which is not available in this virtual machine.\n\
                         \n\
                         This computer appears to be a VM without nested virtualization support.\n\
                         WSL2 cannot run inside VMs that don't support nested virtualization.\n\
                         \n\
                         Options:\n\
                         - Use a physical Windows PC for sandbox installation\n\
                         - Use 'Native' install mode instead (no sandbox, runs directly on host)\n\
                         - Use a cloud VM with nested virtualization (e.g., Azure Dv5 series)"
                    );
                }
            }

            // Stage 2: are WSL + VMP features both enabled?
            if self.is_available().await? {
                return Ok(());
            }
            let runner = LocalProcessRunner::new();
            let wsl_check = runner.exec(
                CommandSpec::new("dism", [
                    "/online", "/get-featureinfo",
                    "/featurename:Microsoft-Windows-Subsystem-Linux", "/English"
                ]).with_timeout(Duration::from_secs(15)),
                CancellationToken::new(),
            ).await;
            let wsl_enabled = wsl_check.as_ref()
                .map(|r| r.stdout.contains("Enabled")).unwrap_or(false);

            let vmp_check = runner.exec(
                CommandSpec::new("dism", [
                    "/online", "/get-featureinfo",
                    "/featurename:VirtualMachinePlatform", "/English"
                ]).with_timeout(Duration::from_secs(15)),
                CancellationToken::new(),
            ).await;
            let vmp_enabled = vmp_check.as_ref()
                .map(|r| r.stdout.contains("Enabled")).unwrap_or(false);

            if wsl_enabled && vmp_enabled {
                // Try setting WSL2 default + kernel update without UAC.
                let _ = runner.exec(
                    CommandSpec::new("wsl", ["--set-default-version", "2"])
                        .with_timeout(Duration::from_secs(10)),
                    CancellationToken::new(),
                ).await;
                let _ = runner.exec(
                    CommandSpec::new("wsl", ["--update"])
                        .with_timeout(Duration::from_secs(120)),
                    CancellationToken::new(),
                ).await;
                tokio::time::sleep(Duration::from_secs(2)).await;
                if self.is_available().await? { return Ok(()); }
            }

            // Stage 3: enable features via UAC. Intentionally visible
            // window so user sees elevation + dism output.
            let install_script = "$ErrorActionPreference = 'Stop';\
                Write-Host 'Enabling Windows Subsystem for Linux...';\
                dism /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart;\
                Write-Host 'Enabling Virtual Machine Platform...';\
                dism /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart;\
                Write-Host 'Installing WSL kernel update...';\
                wsl --update 2>$null;\
                Write-Host 'Done. A restart may be required.';\
                Start-Sleep -Seconds 3";
            let escaped = install_script.replace('\'', "''");
            let outer = format!(
                "Start-Process -FilePath 'powershell' -ArgumentList '-ExecutionPolicy Bypass -Command {escaped}' -Verb RunAs -Wait"
            );
            let elev = runner.exec(
                CommandSpec::new("powershell", ["-Command", outer.as_str()])
                    .with_timeout(Duration::from_secs(10 * 60)),
                CancellationToken::new(),
            ).await;
            if elev.map(|r| r.success()).unwrap_or(false) {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if self.is_available().await? { return Ok(()); }
                anyhow::bail!(
                    "WSL2 installation completed. A system restart is required.\n\
                     Restart your computer, then run this command again."
                );
            }
            anyhow::bail!(
                "WSL2 installation was cancelled or failed.\n\
                 To install manually:\n\
                 1. Open PowerShell as Administrator\n\
                 2. dism /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart\n\
                 3. dism /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart\n\
                 4. Restart your computer\n\
                 5. Run this command again"
            );
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
    async fn ensure_prerequisites_bails_on_non_windows() {
        let b = WslBackend::new("demo");
        let err = b.ensure_prerequisites().await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Windows"),
            "wrong-host bail should mention Windows, got: {msg}"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn destroy_is_noop_on_non_windows() {
        // Non-Windows: is_present returns false, destroy short-circuits Ok.
        let b = WslBackend::new("demo");
        b.destroy().await.unwrap();
    }
}
