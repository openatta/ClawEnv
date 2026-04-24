//! v2-native path helpers. Replaces our former path dep on
//! `clawenv_core::{config::clawenv_root, manager::install_native::*, sandbox::lima_home}`.
//!
//! Keeps the same on-disk layout as v1 so a user can freely run both v1
//! and v2 against the same `~/.clawenv/` tree — only additions live under
//! `~/.clawenv/cache/artifacts/` (our download cache) and
//! `~/.clawenv/v2-instances.toml` (our instance registry).

use std::path::PathBuf;

/// Root directory for all ClawEnv state.
///
/// Resolution order (matches v1 for compatibility):
/// 1. `CLAWENV_HOME` env var (if set — used by tests for isolation)
/// 2. `~/.clawenv`
pub fn clawenv_root() -> PathBuf {
    if let Ok(p) = std::env::var("CLAWENV_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".clawenv")
}

fn home_dir() -> PathBuf {
    // Avoid adding the `dirs` crate just for one call — std is enough.
    #[cfg(unix)]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| {
                let drive = std::env::var_os("HOMEDRIVE");
                let path = std::env::var_os("HOMEPATH");
                match (drive, path) {
                    (Some(d), Some(p)) => {
                        let mut s = std::ffi::OsString::new();
                        s.push(d);
                        s.push(p);
                        Some(s)
                    }
                    _ => None,
                }
            })
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\"))
    }
}

/// `~/.clawenv/node/` — where we unpack the portable Node.js.
pub fn clawenv_node_dir() -> PathBuf {
    clawenv_root().join("node")
}

/// `~/.clawenv/git/` — where we unpack portable Git (dugite / MinGit).
pub fn clawenv_git_dir() -> PathBuf {
    clawenv_root().join("git")
}

/// Lima's working directory. Honors `LIMA_HOME` env var per upstream
/// convention; otherwise `~/.lima/`.
pub fn lima_home() -> PathBuf {
    if let Ok(p) = std::env::var("LIMA_HOME") {
        return PathBuf::from(p);
    }
    home_dir().join(".lima")
}

/// v2's own config directory under `~/.clawenv/v2/`.
pub fn v2_config_dir() -> PathBuf {
    clawenv_root().join("v2")
}

/// v2's instance registry TOML file.
pub fn v2_instances_path() -> PathBuf {
    v2_config_dir().join("instances.toml")
}

/// v2's download cache root.
pub fn v2_cache_root() -> PathBuf {
    clawenv_root().join("cache").join("artifacts")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests in this module mutate `CLAWENV_HOME`. Rust test binaries run
    // tests concurrently within one process, so any two env-touching tests
    // race. Serialize them through a module-level mutex.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn env_override_works() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let original = std::env::var("CLAWENV_HOME").ok();
        // SAFETY: test-local usage, guarded by ENV_LOCK.
        unsafe { std::env::set_var("CLAWENV_HOME", "/tmp/clawenv-test-path"); }
        assert_eq!(clawenv_root(), PathBuf::from("/tmp/clawenv-test-path"));
        match original {
            Some(v) => unsafe { std::env::set_var("CLAWENV_HOME", v) },
            None => unsafe { std::env::remove_var("CLAWENV_HOME") },
        }
    }

    #[test]
    fn node_and_git_dirs_rooted_under_clawenv() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let root = clawenv_root();
        assert!(clawenv_node_dir().starts_with(&root));
        assert!(clawenv_git_dir().starts_with(&root));
        assert!(v2_cache_root().starts_with(&root));
        assert!(v2_instances_path().starts_with(&root));
    }
}
