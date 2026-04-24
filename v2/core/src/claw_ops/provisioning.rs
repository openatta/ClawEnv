//! ClawProvisioning trait — install-time description of a Claw product.
//!
//! Per R3 D2 decision: kept separate from `ClawCli` (which handles
//! CLI-command generation for runtime ops). An install path needs to
//! know "what apk packages, what npm/pip command, what binary name to
//! check"; none of that is CLI-specific.
//!
//! Ported minimally from v1 `core/src/claw/descriptor.rs`. Stage-B
//! features (dashboard pre-build, MCP plugins, config_init_cmd) are
//! deferred to R3.1 per D3.

use serde::{Deserialize, Serialize};

/// How a Claw is installed inside the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    /// `npm install -g <pkg>@<version>`
    Npm,
    /// `pip install --break-system-packages <pkg>[==<version>]`
    Pip,
    /// git clone + `uv pip install -e`. Carries the repo URL and the
    /// pip "extras" spec inline so the trait consumer gets everything
    /// needed in one place.
    GitPip { repo: String, extras: String },
}

/// Static-ish metadata an install pipeline needs about a Claw. Not a
/// trait bound: instances that live in `ClawRegistry` return a concrete
/// `ClawProvisioning` via the `provisioning()` method on the trait
/// above, so we get polymorphism without trait-object gymnastics.
pub trait ClawProvisioning: Send + Sync {
    /// Stable identifier. Matches `ClawCli::id()`.
    fn id(&self) -> &'static str;

    /// Human-readable name (for progress messages).
    fn display_name(&self) -> &'static str;

    /// Whether this claw can run directly on the host (no VM).
    fn supports_native(&self) -> bool;

    /// Binary name after install. Used for version probing.
    fn cli_binary(&self) -> &'static str;

    /// Version-check flag, typically `--version` but occasionally `-V`.
    fn version_flag(&self) -> &'static str { "--version" }

    /// Extra apk packages required on top of the base set
    /// (`git curl bash nodejs npm ttyd openssh build-base python3 procps`).
    /// Hermes for example wants `python3-dev` for native module builds.
    fn sandbox_provision_packages(&self) -> &'static [&'static str] { &[] }

    /// How this claw is installed inside the sandbox.
    fn package_manager(&self) -> PackageManager;

    /// Compose the shell command that installs this claw at `version`
    /// inside a running sandbox. Produced by dispatching on
    /// [`package_manager`](Self::package_manager).
    ///
    /// Default impl handles the three variants; claws with extra quirks
    /// (Hermes needs `fastapi` + `uvicorn` installed via pip after the
    /// git_pip path) can override.
    fn install_cmd(&self, version: &str) -> String {
        default_install_cmd(self, version)
    }

    /// `cli_binary --version` command string (for post-install verify).
    fn version_check_cmd(&self) -> String {
        format!("{} {}", self.cli_binary(), self.version_flag())
    }
}

/// Default dispatch: builds the shell string for each PackageManager
/// variant. Kept as a free fn so it can be called by default impls AND
/// by overrides that want to start from the baseline and append extras.
pub fn default_install_cmd<T: ClawProvisioning + ?Sized>(c: &T, version: &str) -> String {
    match c.package_manager() {
        PackageManager::Npm => {
            // v1 uses --loglevel verbose so `background_script` can
            // poll for progress.
            // Assumes npm_package_name == id; override if it diverges.
            format!(
                "npm install -g --loglevel verbose {}@{}",
                c.cli_binary(),
                version
            )
        }
        PackageManager::Pip => {
            if version == "latest" {
                format!("pip install --break-system-packages {}", c.cli_binary())
            } else {
                format!(
                    "pip install --break-system-packages {}=={}",
                    c.cli_binary(),
                    version
                )
            }
        }
        PackageManager::GitPip { repo, extras } => {
            let dir = format!("/opt/{}", c.id());
            let branch = if version == "latest" {
                "main".to_string()
            } else {
                format!("v{version}")
            };
            let extras_spec = if extras.is_empty() {
                ".".to_string()
            } else {
                format!(".[{extras}]")
            };
            let bin = c.cli_binary();
            // Exact v1 recipe: clone → uv venv → uv pip install -e →
            // symlink binary into /usr/local/bin so PATH sees it.
            format!(
                "git clone --depth 1 --branch {branch} {repo} {dir} \
                 && cd {dir} \
                 && uv venv {dir}/venv --python python3 \
                 && VIRTUAL_ENV={dir}/venv uv pip install -e '{extras_spec}' \
                 && ln -sf {dir}/venv/bin/{bin} /usr/local/bin/{bin}"
            )
        }
    }
}

// ——— Hermes impl ———

pub struct HermesProvisioning;

impl ClawProvisioning for HermesProvisioning {
    fn id(&self) -> &'static str { "hermes" }
    fn display_name(&self) -> &'static str { "Hermes Agent" }
    fn supports_native(&self) -> bool { false }
    fn cli_binary(&self) -> &'static str { "hermes" }
    fn sandbox_provision_packages(&self) -> &'static [&'static str] {
        // From v1 descriptor for Hermes: python3-dev for native wheels,
        // uv because the install recipe relies on it.
        &["python3-dev", "uv"]
    }
    fn package_manager(&self) -> PackageManager {
        PackageManager::GitPip {
            repo: "https://github.com/NousResearch/hermes-agent.git".into(),
            extras: "all".into(),
        }
    }
}

// ——— OpenClaw impl ———

pub struct OpenClawProvisioning;

impl ClawProvisioning for OpenClawProvisioning {
    fn id(&self) -> &'static str { "openclaw" }
    fn display_name(&self) -> &'static str { "OpenClaw" }
    fn supports_native(&self) -> bool { true }
    fn cli_binary(&self) -> &'static str { "openclaw" }
    fn package_manager(&self) -> PackageManager { PackageManager::Npm }
}

// ——— Registry ———

/// Registry entry for install-time lookup. Parallel to
/// [`ClawRegistry::cli_for`](super::ClawRegistry::cli_for) which
/// returns the CLI-generation impl.
pub fn provisioning_for(id: &str) -> Option<Box<dyn ClawProvisioning>> {
    match id {
        "hermes" => Some(Box::new(HermesProvisioning)),
        "openclaw" => Some(Box::new(OpenClawProvisioning)),
        _ => None,
    }
}

/// All known provisionings — for listing / UI.
pub fn all_provisionings() -> Vec<Box<dyn ClawProvisioning>> {
    vec![
        Box::new(HermesProvisioning),
        Box::new(OpenClawProvisioning),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lookup_known_ids() {
        assert!(provisioning_for("openclaw").is_some());
        assert!(provisioning_for("hermes").is_some());
        assert!(provisioning_for("nope").is_none());
    }

    #[test]
    fn openclaw_defaults() {
        let p = OpenClawProvisioning;
        assert_eq!(p.id(), "openclaw");
        assert_eq!(p.cli_binary(), "openclaw");
        assert!(p.supports_native());
        assert!(matches!(p.package_manager(), PackageManager::Npm));
    }

    #[test]
    fn hermes_defaults() {
        let p = HermesProvisioning;
        assert_eq!(p.id(), "hermes");
        assert!(!p.supports_native());
        // Extra apk packages: python3-dev + uv.
        let pkgs = p.sandbox_provision_packages();
        assert!(pkgs.contains(&"python3-dev"));
        assert!(pkgs.contains(&"uv"));
        // GitPip variant.
        match p.package_manager() {
            PackageManager::GitPip { repo, extras } => {
                assert!(repo.contains("github.com"));
                assert_eq!(extras, "all");
            }
            _ => panic!("expected GitPip"),
        }
    }

    // ——— install_cmd dispatch ———

    #[test]
    fn npm_install_cmd_uses_verbose_loglevel() {
        let p = OpenClawProvisioning;
        let cmd = p.install_cmd("1.2.3");
        assert!(cmd.contains("npm install -g"));
        assert!(cmd.contains("--loglevel verbose"));
        assert!(cmd.contains("openclaw@1.2.3"));
    }

    #[test]
    fn npm_install_cmd_passes_latest_keyword() {
        let p = OpenClawProvisioning;
        let cmd = p.install_cmd("latest");
        assert!(cmd.contains("openclaw@latest"));
    }

    #[test]
    fn git_pip_install_cmd_embeds_venv_and_symlink() {
        let p = HermesProvisioning;
        let cmd = p.install_cmd("latest");
        // Branch: "main" for latest.
        assert!(cmd.contains("--branch main"));
        assert!(cmd.contains("git clone"));
        assert!(cmd.contains("/opt/hermes"));
        assert!(cmd.contains("uv venv"));
        assert!(cmd.contains("VIRTUAL_ENV="));
        assert!(cmd.contains("uv pip install -e"));
        // Symlink binary.
        assert!(cmd.contains("ln -sf /opt/hermes/venv/bin/hermes /usr/local/bin/hermes"));
    }

    #[test]
    fn git_pip_install_cmd_version_becomes_vtag() {
        let p = HermesProvisioning;
        let cmd = p.install_cmd("1.0.0");
        assert!(cmd.contains("--branch v1.0.0"));
    }

    #[test]
    fn git_pip_install_cmd_embeds_extras() {
        let p = HermesProvisioning;
        let cmd = p.install_cmd("latest");
        // Extras default for Hermes is "all" → `.[all]`.
        assert!(cmd.contains(".[all]"), "missing extras in cmd: {cmd}");
    }

    // ——— version_check_cmd ———

    #[test]
    fn version_check_cmd_uses_binary_and_flag() {
        let p = OpenClawProvisioning;
        assert_eq!(p.version_check_cmd(), "openclaw --version");
    }

    // ——— default_install_cmd is usable via trait-object ———

    #[test]
    fn dispatch_via_trait_object() {
        let p: Box<dyn ClawProvisioning> = Box::new(OpenClawProvisioning);
        let cmd = p.install_cmd("latest");
        assert!(cmd.contains("openclaw@latest"));
    }
}
