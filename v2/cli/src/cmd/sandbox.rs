use clap::Subcommand;
use clawops_core::sandbox_ops::{BackendKind, LimaOps, PodmanOps, SandboxOps, WslOps};
use clawops_core::{CancellationToken, ProgressSink};

use crate::shared::{new_table, severity_color, Ctx};

#[derive(Subcommand)]
pub enum SandboxCmd {
    /// Show VM status + capabilities.
    Status {
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    /// Start the sandbox VM.
    Start { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Stop the sandbox VM.
    Stop { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Restart.
    Restart { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Port forward management.
    Port {
        #[command(subcommand)] op: PortOp,
    },
    /// Run diagnostics.
    Doctor { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Apply repair recipes for given issue IDs.
    Repair {
        /// Issue IDs to repair (e.g. `vm-not-running`).
        issue_ids: Vec<String>,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    /// Show resource usage.
    Stats { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// List ALL VMs/containers known to the host backend (whether
    /// registered with v2 or not). Useful for discovering orphan VMs.
    List { #[arg(long, value_enum)] backend: Option<BackendSel> },
    /// Rename a sandbox VM. Backend-specific: Lima supports it via
    /// limactl; WSL/Podman need recreate (deferred — bails clean).
    Rename {
        #[arg(long)] from: String,
        #[arg(long)] to: String,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    /// Edit sandbox resource allocation (CPUs, memory, disk). Lima
    /// only for in-place edits; WSL/Podman bail with guidance.
    Edit {
        #[arg(long)] cpus: Option<u32>,
        #[arg(long = "memory-mb")] memory_mb: Option<u32>,
        #[arg(long = "disk-gb")] disk_gb: Option<u32>,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    /// Install host-side prerequisites for the chosen backend
    /// (limactl on macOS, dism/WSL on Windows, podman+uidmap on Linux).
    /// Idempotent — safe to re-run; a no-op when prereqs are already in.
    Prereqs {
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    /// Disk usage of the backend's data dir (Lima ~/.lima, Podman
    /// rootless storage, WSL distro). Reports a human-readable size.
    DiskUsage {
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
}

#[derive(Subcommand)]
pub enum PortOp {
    List {
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    Add {
        host: u16, guest: u16,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
    Remove {
        host: u16,
        #[arg(long, value_enum)] backend: Option<BackendSel>,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum BackendSel { Lima, Wsl2, Podman }

fn pick_default_backend() -> BackendSel {
    if cfg!(target_os = "macos") { BackendSel::Lima }
    else if cfg!(target_os = "windows") { BackendSel::Wsl2 }
    else { BackendSel::Podman }
}

fn ops_for(sel: BackendSel, instance: &str) -> Box<dyn SandboxOps> {
    match sel {
        BackendSel::Lima => Box::new(LimaOps::new(instance)),
        BackendSel::Wsl2 => Box::new(WslOps::new(instance)),
        BackendSel::Podman => Box::new(PodmanOps::new(instance)),
    }
}

fn resolve(backend: Option<BackendSel>) -> BackendSel {
    backend.unwrap_or_else(pick_default_backend)
}

pub async fn run(cmd: SandboxCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        SandboxCmd::Status { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            let s = ops.status().await?;
            ctx.emit_pretty(&s, |st| {
                println!("Backend      : {}", format!("{:?}", st.backend).to_lowercase());
                println!("Instance     : {}", st.instance_name);
                println!("State        : {}", format!("{:?}", st.state).to_lowercase());
                if let Some(c) = st.cpu_cores { println!("CPU cores    : {c}"); }
                if let Some(m) = st.memory_mb { println!("Memory       : {m} MB"); }
                if let Some(d) = st.disk_gb { println!("Disk         : {d} GB"); }
                if let Some(ip) = &st.ip { println!("IP           : {ip}"); }
            })?;
        }
        SandboxCmd::Start { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            ops.start(ProgressSink::noop(), CancellationToken::new()).await?;
            ctx.emit_text("started");
        }
        SandboxCmd::Stop { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            ops.stop(ProgressSink::noop(), CancellationToken::new()).await?;
            ctx.emit_text("stopped");
        }
        SandboxCmd::Restart { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            ops.restart(ProgressSink::noop(), CancellationToken::new()).await?;
            ctx.emit_text("restarted");
        }
        SandboxCmd::Port { op } => {
            match op {
                PortOp::List { backend } => {
                    let ops = ops_for(resolve(backend), &ctx.instance);
                    let ports = ops.list_ports().await?;
                    ctx.emit_pretty(&ports, |rows| {
                        if rows.is_empty() {
                            println!("No port forwards configured.");
                        } else {
                            let mut t = new_table(["host", "guest", "native_id"]);
                            for p in rows {
                                t.add_row([
                                    p.host.to_string(),
                                    p.guest.to_string(),
                                    p.native_id.clone().unwrap_or_else(|| "—".into()),
                                ]);
                            }
                            println!("{t}");
                        }
                    })?;
                }
                PortOp::Add { host, guest, backend } => {
                    let ops = ops_for(resolve(backend), &ctx.instance);
                    ops.add_port(host, guest).await?;
                    ctx.emit_text(format!("added {host} → {guest}"));
                }
                PortOp::Remove { host, backend } => {
                    let ops = ops_for(resolve(backend), &ctx.instance);
                    ops.remove_port(host).await?;
                    ctx.emit_text(format!("removed {host}"));
                }
            }
        }
        SandboxCmd::Doctor { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            let r = ops.doctor().await?;
            ctx.emit_pretty(&r, |rep| {
                println!(
                    "backend={} instance={} issues={}",
                    format!("{:?}", rep.backend).to_lowercase(),
                    rep.instance_name,
                    rep.issues.len(),
                );
                if rep.issues.is_empty() {
                    println!("No issues found.");
                } else {
                    for i in &rep.issues {
                        let sev = format!("{:?}", i.severity);
                        println!("[{}] {} — {}", severity_color(&sev), i.id, i.message);
                        if let Some(hint) = &i.repair_hint {
                            println!("    hint: {hint}");
                        }
                    }
                }
                println!("\nchecked at {}", rep.checked_at);
            })?;
        }
        SandboxCmd::Repair { issue_ids, backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            ops.repair(&issue_ids, ProgressSink::noop()).await?;
            ctx.emit_text(format!("repaired {} issue(s)", issue_ids.len()));
        }
        SandboxCmd::Stats { backend } => {
            let ops = ops_for(resolve(backend), &ctx.instance);
            let s = ops.stats().await?;
            ctx.emit(&s)?;
        }
        SandboxCmd::List { backend } => {
            // Inventory ALL VMs/containers known to the chosen backend,
            // marrying each one with the v2 instance registry to flag
            // managed vs orphan rows. Catches mismatches between user
            // expectation and host state (e.g. a Lima VM left over from
            // a prior install that never made it into the registry).
            let sel = resolve(backend);
            let vms = list_vms_for_backend(sel).await?;
            let resp = clawops_core::wire::SandboxListResponse {
                backend: format!("{:?}", sel).to_lowercase(),
                vms,
            };
            ctx.emit_pretty(&resp, |r| {
                if r.vms.is_empty() {
                    println!("No VMs found.");
                    return;
                }
                let mut t = new_table(["name", "status", "managed", "instance"]);
                for v in &r.vms {
                    t.add_row([
                        v.name.clone(),
                        v.status.clone(),
                        if v.managed { "yes".into() } else { "no".into() },
                        if v.instance_name.is_empty() { "—".into() } else { v.instance_name.clone() },
                    ]);
                }
                println!("{t}");
            })?;
        }
        SandboxCmd::Rename { from, to, backend } => {
            let sel = resolve(backend);
            let ops = ops_for(sel, &from);
            ops.rename(&to).await?;
            ctx.emit_text(format!("renamed {from} → {to}"));
        }
        SandboxCmd::Prereqs { backend } => {
            use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, WslBackend, SandboxBackend};
            use std::sync::Arc;
            let sel = resolve(backend);
            let inst = ctx.instance.clone();
            let b: Arc<dyn SandboxBackend> = match sel {
                BackendSel::Lima => Arc::new(LimaBackend::new(&inst)),
                BackendSel::Wsl2 => Arc::new(WslBackend::new(&inst)),
                BackendSel::Podman => Arc::new(PodmanBackend::new(&inst)),
            };
            b.ensure_prerequisites().await?;
            ctx.emit_text(format!("{:?} prerequisites OK", sel));
        }
        SandboxCmd::DiskUsage { backend } => {
            // du -sh on whichever directory the backend uses for its
            // private state. Cross-platform: macOS/Linux use du(1),
            // Windows uses PowerShell's Get-ChildItem | Measure-Object.
            let sel = resolve(backend);
            let dir = match sel {
                BackendSel::Lima => clawops_core::paths::lima_home(),
                BackendSel::Podman => clawops_core::paths::clawenv_root().join("podman"),
                BackendSel::Wsl2 => clawops_core::paths::clawenv_root().join("wsl"),
            };
            let size = du_human(&dir).await.unwrap_or_else(|_| "unknown".into());
            ctx.emit_pretty(&serde_json::json!({
                "backend": format!("{:?}", sel).to_lowercase(),
                "path": dir,
                "size": size,
            }), |v| {
                println!("{} : {} ({})",
                    v["backend"].as_str().unwrap_or("?"),
                    v["path"].as_str().unwrap_or("?"),
                    v["size"].as_str().unwrap_or("?"));
            })?;
        }
        SandboxCmd::Edit { cpus, memory_mb, disk_gb, backend } => {
            // Per-backend edit: Lima rewrites lima.yaml in place;
            // WSL/Podman delegate to the SandboxOps trait, whose
            // default-bail explains the recreate workflow.
            let sel = resolve(backend);
            if !matches!(sel, BackendSel::Lima) {
                // WSL/Podman: surface a clear, actionable error from
                // the trait's default impl. resize_disk is the most
                // relevant "knob"; cpus/memory follow the same path.
                let ops = ops_for(sel, &ctx.instance);
                if let Some(d) = disk_gb {
                    ops.resize_disk(d).await?;
                }
                anyhow::bail!(
                    "sandbox edit on {sel:?}: in-place cpu/memory edit not supported. \
                     Destroy + recreate the instance with the desired flags, OR for \
                     Podman use `podman update <ctr> --cpus N --memory NNm` directly."
                );
            }
            // Lima path — write the yaml; resize_disk is gated separately
            // because it needs both yaml edit AND a restart, not just yaml.
            let yaml_path = clawops_core::paths::lima_home()
                .join(&ctx.instance).join("lima.yaml");
            if !yaml_path.exists() {
                anyhow::bail!("lima.yaml not found: {}", yaml_path.display());
            }
            let body = tokio::fs::read_to_string(&yaml_path).await?;
            let body = if let Some(c) = cpus {
                rewrite_yaml_scalar(&body, "cpus", &c.to_string())
            } else { body };
            let body = if let Some(m) = memory_mb {
                rewrite_yaml_scalar(&body, "memory", &format!("\"{m}MiB\""))
            } else { body };
            let body = if let Some(d) = disk_gb {
                rewrite_yaml_scalar(&body, "disk", &format!("\"{d}GiB\""))
            } else { body };
            tokio::fs::write(&yaml_path, body).await?;
            ctx.emit_text(format!(
                "Updated {}; restart the VM with `clawcli restart {}` to apply.",
                yaml_path.display(), ctx.instance
            ));
        }
    }
    Ok(())
}

/// Rewrite a top-level `key: value` line in a YAML-ish file. If the
/// key is missing, append it. lima.yaml's `cpus:` and `memory:` are
/// flat top-level scalars, so a line-level rewrite is enough.
fn rewrite_yaml_scalar(body: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(body.len() + 32);
    let mut replaced = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(&format!("{key}:")) && !replaced {
            out.push_str(&format!("{key}: {value}"));
            out.push('\n');
            replaced = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !replaced {
        out.push_str(&format!("{key}: {value}\n"));
    }
    out
}

/// Backend-host inventory → Vec<wire::SandboxVmInfo>.
/// Shells the per-backend tool, parses output, marries with v2 registry
/// to populate `managed` + `instance_name`. Tolerates missing tools
/// (returns empty list rather than error) so the GUI sandbox page can
/// render even before the backend is installed.
async fn list_vms_for_backend(sel: BackendSel)
    -> anyhow::Result<Vec<clawops_core::wire::SandboxVmInfo>>
{
    use clawops_core::common::{CommandRunner, CommandSpec, CancellationToken};
    use clawops_core::runners::LocalProcessRunner;
    use std::time::Duration;
    let runner = LocalProcessRunner::new();
    let (cmd, args): (&str, Vec<&str>) = match sel {
        BackendSel::Lima => ("limactl", vec!["list", "--format", "json"]),
        BackendSel::Podman => ("podman", vec!["ps", "-a", "--format", "json"]),
        BackendSel::Wsl2 => ("wsl", vec!["-l", "-v"]),
    };
    let res = runner.exec(
        CommandSpec::new(cmd, args).with_timeout(Duration::from_secs(5)),
        CancellationToken::new(),
    ).await;
    let stdout = match res {
        Ok(r) if r.success() => r.stdout,
        _ => return Ok(Vec::new()), // backend not installed / no VMs
    };

    // v2 registry → quick lookup for managed flag.
    use clawops_core::instance::InstanceRegistry;
    let reg = InstanceRegistry::with_default_path();
    let registered = reg.list().await.unwrap_or_default();

    let parsed = match sel {
        BackendSel::Lima => parse_lima_list_json(&stdout),
        BackendSel::Podman => parse_podman_ps_json(&stdout),
        BackendSel::Wsl2 => parse_wsl_list(&stdout),
    };

    // Resolve managed/instance_name for each VM by matching against
    // registry's sandbox_instance field.
    let vms = parsed.into_iter().map(|(name, status)| {
        let inst = registered.iter()
            .find(|i| i.sandbox_instance == name || i.name == name);
        let (managed, instance_name) = match inst {
            Some(i) => (true, i.name.clone()),
            None => (false, String::new()),
        };
        clawops_core::wire::SandboxVmInfo {
            name,
            status,
            managed,
            instance_name,
        }
    }).collect();
    Ok(vms)
}

fn parse_lima_list_json(stdout: &str) -> Vec<(String, String)> {
    // limactl list --format json emits one JSON object per VM, line-delimited.
    stdout.lines().filter_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        Some((
            v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            v.get("status").and_then(|x| x.as_str()).unwrap_or("Unknown").to_string(),
        ))
    }).collect()
}

fn parse_podman_ps_json(stdout: &str) -> Vec<(String, String)> {
    // `podman ps -a --format json` emits a single JSON array.
    let v: serde_json::Value = match serde_json::from_str(stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = match v.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter().filter_map(|c| {
        // podman ps json: Names is array of strings; State is "running"/"exited"/etc.
        let name = c.get("Names").and_then(|n| n.as_array())
            .and_then(|a| a.first()).and_then(|x| x.as_str())
            .unwrap_or("").to_string();
        if name.is_empty() { return None; }
        let status = c.get("State").and_then(|x| x.as_str())
            .unwrap_or("Unknown").to_string();
        Some((name, status))
    }).collect()
}

fn parse_wsl_list(stdout: &str) -> Vec<(String, String)> {
    // `wsl -l -v` output:
    //   NAME    STATE    VERSION
    //   Alpine  Running  2
    //   Ubuntu  Stopped  2
    // Skip header line, split by whitespace. Sometimes there's a UTF-16
    // BOM or stray nulls; normalise via filter().
    stdout.lines().skip(1).filter_map(|line| {
        let trimmed = line.trim().trim_start_matches('*').trim();
        if trimmed.is_empty() { return None; }
        let mut parts = trimmed.split_whitespace();
        let name = parts.next()?.to_string();
        let status = parts.next().unwrap_or("Unknown").to_string();
        Some((name, status))
    }).collect()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod list_vm_tests {
    use super::*;

    #[test]
    fn parse_lima_handles_one_per_line() {
        let stdout = r#"{"name":"clawenv-abc","status":"Running"}
{"name":"clawenv-def","status":"Stopped"}"#;
        let r = parse_lima_list_json(stdout);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].0, "clawenv-abc");
        assert_eq!(r[0].1, "Running");
        assert_eq!(r[1].1, "Stopped");
    }

    #[test]
    fn parse_lima_skips_garbage() {
        let r = parse_lima_list_json("not json\n{}");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, "");
    }

    #[test]
    fn parse_wsl_skips_header_and_handles_default_marker() {
        let stdout = "  NAME    STATE      VERSION\n* Alpine  Running    2\n  Ubuntu  Stopped    2\n";
        let r = parse_wsl_list(stdout);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], ("Alpine".into(), "Running".into()));
        assert_eq!(r[1], ("Ubuntu".into(), "Stopped".into()));
    }

    #[test]
    fn parse_podman_array_extracts_names_and_state() {
        let stdout = r#"[{"Names":["openclaw-c1"],"State":"running"},
                        {"Names":["other"],"State":"exited"}]"#;
        let r = parse_podman_ps_json(stdout);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], ("openclaw-c1".into(), "running".into()));
    }
}

/// Human-readable size of `dir`. Returns "0B" for non-existent dirs
/// (caller decides whether to surface that as an error). On Unix
/// shells out to `du -sh`; on Windows to PowerShell. Best-effort —
/// any failure surfaces as Err, callers may fall back to "unknown".
async fn du_human(dir: &std::path::Path) -> anyhow::Result<String> {
    if !dir.exists() {
        return Ok("0B".into());
    }
    #[cfg(unix)]
    {
        let out = tokio::process::Command::new("du")
            .args(["-sh", &dir.to_string_lossy()])
            .output()
            .await?;
        let s = String::from_utf8_lossy(&out.stdout);
        Ok(s.split_whitespace().next().unwrap_or("unknown").to_string())
    }
    #[cfg(windows)]
    {
        let script = format!(
            "(Get-ChildItem -Recurse -Force -ErrorAction SilentlyContinue '{}' | \
              Measure-Object -Property Length -Sum).Sum",
            dir.display()
        );
        let out = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .await?;
        let bytes: u64 = String::from_utf8_lossy(&out.stdout)
            .trim().parse().unwrap_or(0);
        Ok(humanize_bytes(bytes))
    }
}

#[cfg(windows)]
fn humanize_bytes(b: u64) -> String {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < UNITS.len() {
        v /= 1024.0; i += 1;
    }
    format!("{:.1}{}", v, UNITS[i])
}

// Suppress unused import warning for BackendKind — kept in re-exports so
// external tests can reference it; no direct use in this file.
#[allow(dead_code)]
const _: Option<BackendKind> = None;
