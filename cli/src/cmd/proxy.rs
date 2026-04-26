//! `clawops proxy ...` — surface the new proxy/credentials modules.
//!
//! Subcommands:
//! - `resolve --scope installer|native|sandbox [--backend ...]` —
//!   show the triple that would be applied right now.
//! - `set-password` — write a global proxy password into the
//!   credentials vault (reads from stdin or `--stdin`).
//! - `clear-password` — delete the global proxy password.
//! - `apply --backend ...` — write the triple into the sandbox.
//! - `clear --backend ...` — remove proxy files from the sandbox.

use std::io::Read;
use std::sync::Arc;

use clap::{Subcommand, ValueEnum};
use clawops_core::credentials;
use clawops_core::proxy::{self, ProxyConfig, Scope};
use clawops_core::sandbox_backend::{LimaBackend, PodmanBackend, SandboxBackend, WslBackend};
use clawops_core::sandbox_ops::BackendKind;

use crate::shared::Ctx;

#[derive(Subcommand)]
pub enum ProxyCmd {
    /// Show the ProxyTriple that would be applied for a given scope.
    Resolve {
        #[arg(long, value_enum, default_value_t = ScopeSel::Installer)]
        scope: ScopeSel,
        /// Only meaningful for --scope sandbox.
        #[arg(long, value_enum)]
        backend: Option<BackendSel>,
    },
    /// Store the global proxy password. Password is read from stdin
    /// (or the first line of stdin when `--stdin` is set).
    SetPassword {
        #[arg(long)] stdin: bool,
    },
    /// Delete the global proxy password from the keychain.
    ClearPassword,
    /// Write /etc/environment + /etc/profile.d/proxy.sh inside the sandbox.
    Apply {
        #[arg(long, value_enum)]
        backend: Option<BackendSel>,
    },
    /// Remove the proxy files written by `apply`.
    Clear {
        #[arg(long, value_enum)]
        backend: Option<BackendSel>,
    },
    /// Read the effective proxy for an instance (resolves global config
    /// + env vars to the triple that would apply right now).
    Get {
        /// Instance name. Required so we can pick the right backend
        /// (Lima vs Podman vs WSL → host.* loopback rewrite).
        name: String,
    },
    /// Write the global proxy config (`[clawenv.proxy]` in config.toml)
    /// and optionally apply it to a running instance's VM.
    Set {
        /// Instance to apply to (skip with --no-apply for "config only").
        name: Option<String>,
        /// e.g. `http://user:pass@proxy.corp:3128`. Empty string disables.
        #[arg(long)] url: String,
        /// Override no_proxy list. Default: keep whatever is in config.
        #[arg(long = "no-proxy")] no_proxy: Option<String>,
        /// Don't push to the VM, only persist global config.
        #[arg(long)] no_apply: bool,
    },
    /// Probe an instance VM for the `/etc/profile.d/proxy.sh` baked-in
    /// proxy file. Returns whether the file is present and (optionally)
    /// its contents.
    Check {
        name: String,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum ScopeSel {
    Installer,
    Native,
    Sandbox,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum BackendSel {
    Lima,
    Wsl2,
    Podman,
}

impl From<BackendSel> for BackendKind {
    fn from(b: BackendSel) -> Self {
        match b {
            BackendSel::Lima => BackendKind::Lima,
            BackendSel::Wsl2 => BackendKind::Wsl2,
            BackendSel::Podman => BackendKind::Podman,
        }
    }
}

fn pick_default_backend() -> BackendSel {
    if cfg!(target_os = "macos") { BackendSel::Lima }
    else if cfg!(target_os = "windows") { BackendSel::Wsl2 }
    else { BackendSel::Podman }
}

fn backend_arc(sel: BackendSel, instance: &str) -> Arc<dyn SandboxBackend> {
    match sel {
        BackendSel::Lima => Arc::new(LimaBackend::new(instance)),
        BackendSel::Wsl2 => Arc::new(WslBackend::new(instance)),
        BackendSel::Podman => Arc::new(PodmanBackend::new(instance)),
    }
}

pub async fn run(cmd: ProxyCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        ProxyCmd::Resolve { scope, backend } => {
            // v2 doesn't own config.toml loading yet, so use defaults.
            // Env vars still flow through Scope::resolve.
            let cfg = ProxyConfig::default();
            let triple = match scope {
                ScopeSel::Installer => Scope::Installer.resolve(&cfg, None).await,
                ScopeSel::Native => Scope::RuntimeNative.resolve(&cfg, None).await,
                ScopeSel::Sandbox => {
                    let b: BackendKind = backend.unwrap_or_else(pick_default_backend).into();
                    Scope::RuntimeSandbox { backend: b, instance: None }
                        .resolve(&cfg, None)
                        .await
                }
            };
            ctx.emit_pretty(&triple, |opt| match opt {
                Some(t) => {
                    println!("http     : {}", t.http);
                    println!("https    : {}", t.https);
                    println!("no_proxy : {}", t.no_proxy);
                    println!("source   : {:?}", t.source);
                }
                None => println!("no proxy configured for this scope"),
            })?;
        }
        ProxyCmd::SetPassword { stdin } => {
            let mut pw = String::new();
            if stdin {
                std::io::stdin().read_to_string(&mut pw)?;
                if pw.ends_with('\n') { pw.pop(); }
                if pw.ends_with('\r') { pw.pop(); }
            } else {
                // Interactive: read one line without echo would need
                // rpassword; for now we accept --stdin as the scripted
                // path and tell interactive users to pipe it in.
                anyhow::bail!("pass the password on stdin: `echo -n SECRET | clawcli proxy set-password --stdin`");
            }
            credentials::store_proxy_password(&pw)?;
            ctx.emit_text("proxy password stored");
        }
        ProxyCmd::ClearPassword => {
            credentials::delete_proxy_password()?;
            ctx.emit_text("proxy password cleared");
        }
        ProxyCmd::Apply { backend } => {
            let b_kind: BackendKind = backend.unwrap_or_else(pick_default_backend).into();
            let cfg = ProxyConfig::default();
            let triple = Scope::RuntimeSandbox { backend: b_kind, instance: None }
                .resolve(&cfg, None)
                .await
                .ok_or_else(|| anyhow::anyhow!(
                    "no proxy configured — set HTTP_PROXY or configure global proxy first"
                ))?;
            let sel = backend.unwrap_or_else(pick_default_backend);
            let b = backend_arc(sel, &ctx.instance);
            proxy::apply::apply_to_sandbox(&b, &triple).await?;
            ctx.emit_text("proxy applied to sandbox");
        }
        ProxyCmd::Clear { backend } => {
            let sel = backend.unwrap_or_else(pick_default_backend);
            let b = backend_arc(sel, &ctx.instance);
            proxy::apply::clear_sandbox_proxy(&b).await?;
            ctx.emit_text("proxy files removed from sandbox");
        }
        ProxyCmd::Get { name } => {
            // Resolve effective proxy for an instance — use the instance's
            // backend kind so loopback rewrites (Lima→host.lima.internal,
            // Podman→host.containers.internal, etc.) apply correctly.
            use clawops_core::instance::{InstanceRegistry, SandboxKind};
            let reg = InstanceRegistry::with_default_path();
            let inst = reg.find(&name).await?
                .ok_or_else(|| anyhow::anyhow!("instance `{name}` not found"))?;
            let cfg = clawops_core::config_loader::load_global()
                .map_err(|e| anyhow::anyhow!("load config: {e}"))?;
            let scope = match inst.backend {
                SandboxKind::Native => Scope::RuntimeNative,
                SandboxKind::Lima => Scope::RuntimeSandbox {
                    backend: BackendKind::Lima, instance: None },
                SandboxKind::Wsl2 => Scope::RuntimeSandbox {
                    backend: BackendKind::Wsl2, instance: None },
                SandboxKind::Podman => Scope::RuntimeSandbox {
                    backend: BackendKind::Podman, instance: None },
            };
            let triple = scope.resolve(&cfg.proxy, None).await;
            ctx.emit_pretty(&serde_json::json!({
                "instance": inst.name,
                "backend": inst.backend.as_str(),
                "configured": cfg.proxy.enabled,
                "effective": triple,
            }), |v| {
                println!("Instance   : {}", v["instance"].as_str().unwrap_or("?"));
                println!("Configured : {}", v["configured"].as_bool().unwrap_or(false));
                if let Some(t) = v.get("effective").filter(|x| !x.is_null()) {
                    println!("http       : {}", t["http"].as_str().unwrap_or("?"));
                    println!("https      : {}", t["https"].as_str().unwrap_or("?"));
                    println!("no_proxy   : {}", t["no_proxy"].as_str().unwrap_or("?"));
                } else {
                    println!("(no proxy resolves for this instance)");
                }
            })?;
        }
        ProxyCmd::Set { name, url, no_proxy, no_apply } => {
            // Write [clawenv.proxy] then optionally apply to a running VM.
            let mut cfg = clawops_core::config_loader::load_global()
                .map_err(|e| anyhow::anyhow!("load config: {e}"))?
                .proxy;
            if url.is_empty() {
                cfg.enabled = false;
            } else {
                cfg.enabled = true;
                cfg.http_proxy = url.clone();
                cfg.https_proxy = url.clone();
            }
            if let Some(np) = no_proxy {
                cfg.no_proxy = np;
            }
            clawops_core::config_loader::save_proxy_section(&cfg)
                .map_err(|e| anyhow::anyhow!("save proxy: {e}"))?;

            if no_apply {
                ctx.emit_text("proxy config saved (no apply)");
                return Ok(());
            }
            let Some(n) = name else {
                ctx.emit_text("proxy config saved (no instance to apply to)");
                return Ok(());
            };
            // Resolve & apply
            use clawops_core::instance::{InstanceRegistry, SandboxKind};
            let reg = InstanceRegistry::with_default_path();
            let inst = reg.find(&n).await?
                .ok_or_else(|| anyhow::anyhow!("instance `{n}` not found"))?;
            let triple = match inst.backend {
                SandboxKind::Native => Scope::RuntimeNative.resolve(&cfg, None).await,
                SandboxKind::Lima => Scope::RuntimeSandbox {
                    backend: BackendKind::Lima, instance: None,
                }.resolve(&cfg, None).await,
                SandboxKind::Wsl2 => Scope::RuntimeSandbox {
                    backend: BackendKind::Wsl2, instance: None,
                }.resolve(&cfg, None).await,
                SandboxKind::Podman => Scope::RuntimeSandbox {
                    backend: BackendKind::Podman, instance: None,
                }.resolve(&cfg, None).await,
            };
            if matches!(inst.backend, SandboxKind::Native) {
                ctx.emit_text("proxy saved (native instance — host env var, no VM apply)");
                return Ok(());
            }
            let triple = triple.ok_or_else(||
                anyhow::anyhow!("proxy resolved to None for instance `{n}` — set url first")
            )?;
            let target = if inst.sandbox_instance.is_empty() {
                inst.name.clone()
            } else { inst.sandbox_instance.clone() };
            let b: Arc<dyn SandboxBackend> = match inst.backend {
                SandboxKind::Lima => Arc::new(LimaBackend::new(&target)),
                SandboxKind::Wsl2 => Arc::new(WslBackend::new(&target)),
                SandboxKind::Podman => Arc::new(PodmanBackend::new(&target)),
                SandboxKind::Native => unreachable!(),
            };
            proxy::apply::apply_to_sandbox(&b, &triple).await?;
            ctx.emit_text("proxy applied to sandbox");
        }
        ProxyCmd::Check { name } => {
            // Probe /etc/profile.d/proxy.sh inside the VM. Return shape:
            // {present: bool, contents?: string}.
            use clawops_core::instance::{InstanceRegistry, SandboxKind};
            let reg = InstanceRegistry::with_default_path();
            let inst = reg.find(&name).await?
                .ok_or_else(|| anyhow::anyhow!("instance `{name}` not found"))?;
            if matches!(inst.backend, SandboxKind::Native) {
                ctx.emit_pretty(&serde_json::json!({
                    "instance": inst.name,
                    "backend": "native",
                    "present": false,
                    "reason": "native instances don't have a baked-in proxy file",
                }), |v| {
                    println!("(native — no /etc/profile.d/proxy.sh): {}",
                        v["reason"].as_str().unwrap_or("?"));
                })?;
                return Ok(());
            }
            let target = if inst.sandbox_instance.is_empty() {
                inst.name.clone()
            } else { inst.sandbox_instance.clone() };
            let b: Arc<dyn SandboxBackend> = match inst.backend {
                SandboxKind::Lima => Arc::new(LimaBackend::new(&target)),
                SandboxKind::Wsl2 => Arc::new(WslBackend::new(&target)),
                SandboxKind::Podman => Arc::new(PodmanBackend::new(&target)),
                SandboxKind::Native => unreachable!(),
            };
            let probe = b.exec_argv(&[
                "sh", "-c",
                "test -f /etc/profile.d/proxy.sh && cat /etc/profile.d/proxy.sh || echo '__ABSENT__'",
            ]).await;
            let (present, contents) = match probe {
                Ok(s) if s.trim() == "__ABSENT__" => (false, String::new()),
                Ok(s) => (true, s),
                Err(e) => {
                    return Err(anyhow::anyhow!("probe failed: {e}"));
                }
            };
            ctx.emit_pretty(&serde_json::json!({
                "instance": inst.name,
                "backend": inst.backend.as_str(),
                "present": present,
                "contents": contents,
            }), |v| {
                println!("Instance : {}", v["instance"].as_str().unwrap_or("?"));
                println!("Present  : {}", v["present"].as_bool().unwrap_or(false));
                if v["present"].as_bool().unwrap_or(false) {
                    println!("---\n{}", v["contents"].as_str().unwrap_or(""));
                }
            })?;
        }
    }
    Ok(())
}
