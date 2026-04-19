mod models;
mod manager;
pub mod keychain;

use std::path::PathBuf;

/// Root directory for all clawenv state (config, node, git, lima, caches,
/// workspaces). Defaults to `~/.clawenv` but can be overridden via the
/// `CLAWENV_HOME` env var — used by E2E tests to isolate state into a
/// scratch directory instead of the user's real home.
///
/// All code that needs `~/.clawenv/<subdir>` MUST go through this helper
/// rather than hard-coding `dirs::home_dir().join(".clawenv")`. macOS
/// scripts can also get isolation via `$HOME` override, but that doesn't
/// work on Windows (SHGetKnownFolderPath ignores env) — CLAWENV_HOME is
/// the cross-platform answer.
pub fn clawenv_root() -> PathBuf {
    if let Ok(custom) = std::env::var("CLAWENV_HOME") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".clawenv")
}
pub mod mirrors;
pub mod mirrors_asset;
pub mod proxy;
pub mod proxy_resolver;
#[cfg(test)]
mod tests;

pub use models::*;
pub use manager::ConfigManager;
