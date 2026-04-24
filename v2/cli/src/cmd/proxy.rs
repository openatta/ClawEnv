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
                anyhow::bail!("pass the password on stdin: `echo -n SECRET | clawops proxy set-password --stdin`");
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
    }
    Ok(())
}
