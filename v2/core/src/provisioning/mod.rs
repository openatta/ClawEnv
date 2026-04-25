//! Provisioning — the pieces that set up a freshly-created sandbox:
//! mirror configuration, long-running install scripts, in-VM connectivity
//! probes. Runs after `SandboxOps::start()` returns OK, before the first
//! `claw install` invocation.
//!
//! This module is R3's home. The trait `SandboxBackend` intentionally
//! stays narrow — provisioning is a layer above it that composes
//! backend.exec_argv with structured config.

pub mod mirrors;
pub mod background;
pub mod templates;
pub mod dashboard;
pub mod mcp;

pub use mirrors::{apply_mirrors, MirrorsConfig, DEFAULT_ALPINE_REPO, DEFAULT_NPM_REGISTRY};
pub use background::{
    run_background_script, BackgroundScriptOpts, BackgroundScriptReport,
};
pub use templates::{
    render_lima_yaml, render_podman_build_args, render_wsl_provision_script,
    CreateOpts, LIMA_TEMPLATE, PODMAN_CONTAINERFILE,
};
