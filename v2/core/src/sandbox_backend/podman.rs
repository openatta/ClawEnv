//! Podman backend — execute via `podman exec <container>`.
//!
//! Container ports are baked in at `podman run` time; runtime port-edit is
//! not supported (matches v2 SandboxOps behavior).

use std::time::Duration;

use async_trait::async_trait;

use crate::common::{try_exec, CancellationToken, CommandRunner, CommandSpec};
use crate::paths::v2_config_dir;
use crate::provisioning::{render_podman_build_args, CreateOpts, PODMAN_CONTAINERFILE};
use crate::runners::LocalProcessRunner;

use super::{ResourceStats, SandboxBackend};

pub struct PodmanBackend {
    instance: String,
    runner: LocalProcessRunner,
}

impl PodmanBackend {
    pub fn new(instance: impl Into<String>) -> Self {
        Self {
            instance: instance.into(),
            runner: LocalProcessRunner::new(),
        }
    }
}

#[async_trait]
impl SandboxBackend for PodmanBackend {
    fn name(&self) -> &str { "Podman" }
    fn instance(&self) -> &str { &self.instance }

    async fn is_available(&self) -> anyhow::Result<bool> {
        // Tolerate `podman` not being installed — report "not available".
        let spec = CommandSpec::new("podman",
            ["container", "inspect", &self.instance, "--format", "{{.State.Status}}"])
            .with_timeout(Duration::from_secs(5));
        let Some(res) = try_exec(&self.runner, spec, CancellationToken::new()).await?
            else { return Ok(false); };
        if !res.success() { return Ok(false); }
        Ok(res.stdout.trim() == "running")
    }

    async fn is_present(&self) -> anyhow::Result<bool> {
        // `podman container exists` exits 0 when the container is defined,
        // regardless of run state. Tolerates podman-missing.
        let spec = CommandSpec::new("podman", ["container", "exists", &self.instance])
            .with_timeout(Duration::from_secs(5));
        let Some(res) = try_exec(&self.runner, spec, CancellationToken::new()).await?
            else { return Ok(false); };
        Ok(res.success())
    }

    async fn ensure_prerequisites(&self) -> anyhow::Result<()> {
        // Wrong-host gate: Podman is the Linux backend in v2.
        if !cfg!(target_os = "linux") {
            anyhow::bail!(
                "Podman is the Linux sandbox backend; on this host use Lima (macOS) or WSL2 (Windows)"
            );
        }

        // Already installed? `podman --version` exits 0 → done.
        let probe = CommandSpec::new("podman", ["--version"])
            .with_timeout(Duration::from_secs(3));
        if let Some(res) = try_exec(&self.runner, probe, CancellationToken::new()).await? {
            if res.success() {
                return Ok(());
            }
        }

        // Detect package manager and try auto-install via sudo. Order
        // mirrors v1 sandbox/podman.rs:154-159 — apt, dnf, pacman, zypper.
        let candidates: &[(&str, &[&str])] = &[
            ("apt-get", &["install", "-y", "podman"]),
            ("dnf",     &["install", "-y", "podman"]),
            ("pacman",  &["-S", "--noconfirm", "podman"]),
            ("zypper",  &["install", "-y", "podman"]),
        ];

        for (pm, args) in candidates {
            // `which <pm>` exits 0 when present.
            let which = self.runner.exec(
                CommandSpec::new("which", [*pm])
                    .with_timeout(Duration::from_secs(3)),
                CancellationToken::new(),
            ).await;
            let has_pm = which.map(|r| r.success()).unwrap_or(false);
            if !has_pm { continue; }

            let mut full_args: Vec<&str> = vec![*pm];
            full_args.extend_from_slice(args);
            let install = self.runner.exec(
                CommandSpec::new("sudo", full_args)
                    .with_timeout(Duration::from_secs(5 * 60)),
                CancellationToken::new(),
            ).await?;
            if install.success() {
                // Re-probe to confirm.
                let reprobe = CommandSpec::new("podman", ["--version"])
                    .with_timeout(Duration::from_secs(3));
                if let Some(res) = try_exec(&self.runner, reprobe, CancellationToken::new()).await? {
                    if res.success() {
                        return Ok(());
                    }
                }
            }
            // This pm tried but didn't yield a working podman — try next.
        }

        anyhow::bail!(
            "Could not install Podman automatically.\n\
             Please install manually:\n\
             - Fedora/RHEL: sudo dnf install podman\n\
             - Ubuntu/Debian: sudo apt install podman\n\
             - Arch:         sudo pacman -S podman\n\
             - openSUSE:     sudo zypper install podman\n\
             See https://podman.io/docs/installation"
        )
    }

    async fn create(&self, opts: &CreateOpts) -> anyhow::Result<()> {
        // Idempotent: if the container already exists, don't re-provision.
        if self.is_present().await.unwrap_or(false) {
            return Ok(());
        }

        // Stage the Containerfile in a per-instance build context dir.
        // `podman build` wants a filesystem path, not stdin.
        let ctx_dir = v2_config_dir().join("podman-build").join(&self.instance);
        tokio::fs::create_dir_all(&ctx_dir).await
            .map_err(|e| anyhow::anyhow!(
                "create podman build dir {}: {e}", ctx_dir.display()
            ))?;
        let cf_path = ctx_dir.join("Containerfile");
        tokio::fs::write(&cf_path, PODMAN_CONTAINERFILE).await
            .map_err(|e| anyhow::anyhow!(
                "write Containerfile {}: {e}", cf_path.display()
            ))?;

        // Ensure host workspace exists before we bind-mount it.
        tokio::fs::create_dir_all(&opts.workspace_dir).await
            .map_err(|e| anyhow::anyhow!(
                "create workspace {}: {e}", opts.workspace_dir.display()
            ))?;

        // 1) `podman build` — composes image from template with --build-arg.
        let mut build_args = render_podman_build_args(opts);
        build_args.push("-f".into());
        build_args.push(
            cf_path.to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 Containerfile path"))?
                .to_string(),
        );
        build_args.push(
            ctx_dir.to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 build context"))?
                .to_string(),
        );
        let build_refs: Vec<&str> = build_args.iter().map(|s| s.as_str()).collect();
        let spec = CommandSpec::new("podman", build_refs)
            .with_timeout(Duration::from_secs(15 * 60));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!(
                "podman build for {}: exit {}\nstderr:\n{}",
                self.instance, res.exit_code, res.stderr
            );
        }

        // 2) `podman run -d` — starts the container with workspace mount
        //    + port publish + keep-id userns (CLAUDE.md rule: Podman
        //    rootless needs keep-id and :Z on bind mounts).
        let image = format!("clawenv/{}:latest", self.instance);
        let port_pub = format!("{}:{}", opts.gateway_port, opts.gateway_port);
        let workspace_mount = format!(
            "{}:/workspace:Z",
            opts.workspace_dir.display()
        );
        let run_args: Vec<String> = vec![
            "run".into(), "-d".into(),
            "--name".into(), self.instance.clone(),
            "--userns=keep-id".into(),
            "-p".into(), port_pub,
            "-v".into(), workspace_mount,
            image,
        ];
        let run_refs: Vec<&str> = run_args.iter().map(|s| s.as_str()).collect();
        let spec = CommandSpec::new("podman", run_refs)
            .with_timeout(Duration::from_secs(60));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!(
                "podman run for {}: exit {}\nstderr:\n{}",
                self.instance, res.exit_code, res.stderr
            );
        }
        Ok(())
    }

    async fn destroy(&self) -> anyhow::Result<()> {
        if !self.is_present().await.unwrap_or(false) {
            return Ok(());
        }
        // `rm -f` stops+removes; tolerate nonexistence (-f is idempotent-ish).
        let _ = self.runner.exec(
            CommandSpec::new("podman", ["rm", "-f", &self.instance])
                .with_timeout(Duration::from_secs(30)),
            CancellationToken::new(),
        ).await;
        // Remove the image we built for this instance. Best-effort — a
        // shared image across multiple instances isn't our model today.
        let image = format!("clawenv/{}:latest", self.instance);
        let _ = self.runner.exec(
            CommandSpec::new("podman", ["rmi", "-f", image.as_str()])
                .with_timeout(Duration::from_secs(30)),
            CancellationToken::new(),
        ).await;
        // Clean up our build context dir.
        let ctx_dir = v2_config_dir().join("podman-build").join(&self.instance);
        let _ = tokio::fs::remove_dir_all(&ctx_dir).await;
        Ok(())
    }

    async fn start(&self) -> anyhow::Result<()> {
        let spec = CommandSpec::new("podman", ["start", &self.instance])
            .with_timeout(Duration::from_secs(30));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("podman start {} failed: {}", self.instance, res.stderr);
        }
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        let spec = CommandSpec::new("podman", ["stop", &self.instance])
            .with_timeout(Duration::from_secs(30));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("podman stop {} failed: {}", self.instance, res.stderr);
        }
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> anyhow::Result<String> {
        let spec = CommandSpec::new("podman",
            ["exec", &self.instance, "sh", "-c", cmd])
            .with_timeout(Duration::from_secs(5 * 60));
        let res = self.runner.exec(spec, CancellationToken::new()).await?;
        if !res.success() {
            anyhow::bail!("podman exec failed (exit {}): {}", res.exit_code, res.stderr);
        }
        Ok(res.stdout)
    }

    async fn stats(&self) -> anyhow::Result<ResourceStats> {
        // `podman stats --no-stream --format json <name>`. Tolerates podman-missing.
        let spec = CommandSpec::new("podman",
            ["stats", "--no-stream", "--format", "json", &self.instance])
            .with_timeout(Duration::from_secs(10));
        let Some(res) = try_exec(&self.runner, spec, CancellationToken::new()).await?
            else { return Ok(ResourceStats::default()); };
        if !res.success() {
            return Ok(ResourceStats::default());
        }
        let trimmed = res.stdout.trim();
        if trimmed.is_empty() { return Ok(ResourceStats::default()); }
        let v: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => return Ok(ResourceStats::default()),
        };
        // podman stats returns an array of objects.
        let row = v.as_array().and_then(|a| a.first()).cloned().unwrap_or(v);
        let cpu_percent = parse_cpu_percent(
            row["cpu_percent"].as_str().unwrap_or("")
        );
        let memory_used_mb = row["mem_usage"].as_u64().map(|b| b / 1024 / 1024).unwrap_or(0);
        let memory_limit_mb = row["mem_limit"].as_u64().map(|b| b / 1024 / 1024).unwrap_or(0);
        Ok(ResourceStats { cpu_percent, memory_used_mb, memory_limit_mb })
    }

    async fn edit_port_forwards(&self, _forwards: &[(u16, u16)]) -> anyhow::Result<()> {
        anyhow::bail!("Podman ports are set at container creation; edit not supported")
    }

    fn supports_rename(&self) -> bool { false }
    fn supports_resource_edit(&self) -> bool { false }
    fn supports_port_edit(&self) -> bool { false }
}

fn parse_cpu_percent(s: &str) -> f32 {
    // Format: "12.34%" (new) or "0.05" (old). Strip "%" if present.
    s.trim().trim_end_matches('%').parse::<f32>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_and_name() {
        let b = PodmanBackend::new("demo");
        assert_eq!(b.name(), "Podman");
        assert_eq!(b.instance(), "demo");
    }

    #[test]
    fn parse_cpu_variants() {
        assert!((parse_cpu_percent("12.5%") - 12.5).abs() < 0.01);
        assert!((parse_cpu_percent("0.05") - 0.05).abs() < 0.001);
        assert_eq!(parse_cpu_percent("garbage"), 0.0);
    }

    #[cfg(not(target_os = "linux"))]
    #[tokio::test]
    async fn ensure_prerequisites_bails_on_non_linux() {
        let b = PodmanBackend::new("demo");
        let err = b.ensure_prerequisites().await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Linux"),
            "wrong-host bail should mention Linux, got: {msg}"
        );
    }
}
