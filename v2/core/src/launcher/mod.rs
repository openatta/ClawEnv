//! Launch-state detection — what page should the GUI open on startup?
//!
//! Lifted from v1 `core/src/launcher.rs` (P1-a). v2 differences:
//!
//! - v1 reads `[[instances]]` from `~/.clawenv/config.toml` via
//!   `ConfigManager`. v2 has its own [`InstanceRegistry`] at
//!   `<clawenv_root>/v2/instances.toml`. v1 config.toml is also
//!   probed (we keep first-run detection backward-compatible: if
//!   either file exists, we're past first-run).
//! - v1's `LaunchState::UpgradeAvailable` was already commented out
//!   on the launcher path ("skip upgrade check at launch — user can
//!   check from ClawPage. This avoids 3s network delay on every
//!   startup."). v2 drops the variant entirely.
//! - v1's `post_install_start` (auto-start daemon after install)
//!   depends on monitor + claw descriptor's gateway_cmd — those land
//!   in P1-j. Not ported here.

use serde::{Deserialize, Serialize};

use crate::common::OpsError;
use crate::config_loader::default_config_path;
use crate::instance::{InstanceConfig, InstanceRegistry};
use crate::paths::v2_instances_path;

/// Decision the launcher hands back to the GUI/CLI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchState {
    /// First-time run — no config file at all. Show onboarding.
    FirstRun,
    /// User has run before but no claw installed yet. Show install wizard.
    NotInstalled,
    /// Has installed instances. Show main window with the list.
    Ready { instances: Vec<InstanceConfig> },
}

/// Detect launch state by probing config + registry on disk.
///
/// Decision tree:
/// 1. Neither v1 `~/.clawenv/config.toml` nor v2 instances.toml
///    exists → `FirstRun`
/// 2. Some config present but registry has zero instances →
///    `NotInstalled`
/// 3. Registry has at least one instance → `Ready { instances }`
///
/// **Cheap**: only file-existence checks + a single TOML parse.
/// No network, no exec into VMs. Safe to call on every Tauri startup.
pub async fn detect_launch_state() -> Result<LaunchState, OpsError> {
    let v1_config = default_config_path();
    let v2_inst = v2_instances_path();

    if !v1_config.exists() && !v2_inst.exists() {
        return Ok(LaunchState::FirstRun);
    }

    let registry = InstanceRegistry::with_default_path();
    let instances = registry.list().await?;

    if instances.is_empty() {
        return Ok(LaunchState::NotInstalled);
    }

    Ok(LaunchState::Ready { instances })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    use crate::instance::SandboxKind;

    // CLAWENV_HOME is process-global; serialize tests that mutate it.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[allow(clippy::await_holding_lock)]
    async fn run_with_home<F, Fut, T>(home: &std::path::Path, f: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("CLAWENV_HOME").ok();
        unsafe { std::env::set_var("CLAWENV_HOME", home); }
        let r = f().await;
        match prev {
            Some(v) => unsafe { std::env::set_var("CLAWENV_HOME", v) },
            None => unsafe { std::env::remove_var("CLAWENV_HOME") },
        }
        r
    }

    #[tokio::test]
    async fn first_run_when_no_config_and_no_registry() {
        let tmp = TempDir::new().unwrap();
        let state = run_with_home(tmp.path(), detect_launch_state).await.unwrap();
        assert_eq!(state, LaunchState::FirstRun);
    }

    #[tokio::test]
    async fn not_installed_when_v1_config_exists_but_registry_empty() {
        // v1 left a config.toml but user hasn't created any v2 instance yet.
        // This is the "user upgraded clawcli but didn't migrate" case.
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("config.toml"), "[clawenv]\n").unwrap();
        let state = run_with_home(tmp.path(), detect_launch_state).await.unwrap();
        assert_eq!(state, LaunchState::NotInstalled);
    }

    #[tokio::test]
    async fn not_installed_when_v2_instances_file_exists_but_empty() {
        // v2 instances.toml present but no [[instance]] entries — happens
        // after `clawcli instance destroy` of the last instance.
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("v2")).unwrap();
        std::fs::write(tmp.path().join("v2/instances.toml"), "").unwrap();
        let state = run_with_home(tmp.path(), detect_launch_state).await.unwrap();
        assert_eq!(state, LaunchState::NotInstalled);
    }

    #[tokio::test]
    async fn ready_when_registry_has_instances() {
        let tmp = TempDir::new().unwrap();
        // Pre-seed v2 instances.toml by writing the file directly —
        // avoids the nested-runtime problem of block_on inside a
        // #[tokio::test], and exercises the same parser the real
        // `InstanceRegistry::list` uses.
        std::fs::create_dir_all(tmp.path().join("v2")).unwrap();
        std::fs::write(
            tmp.path().join("v2/instances.toml"),
            r#"[[instance]]
name = "demo"
claw = "openclaw"
backend = "lima"
sandbox_instance = "demo"
created_at = "2026-01-01T00:00:00+00:00"

[[instance.ports]]
host = 3000
guest = 3000
label = "gateway"
"#,
        ).unwrap();

        let state = run_with_home(tmp.path(), detect_launch_state).await.unwrap();
        match state {
            LaunchState::Ready { instances } => {
                assert_eq!(instances.len(), 1);
                assert_eq!(instances[0].name, "demo");
                assert_eq!(instances[0].claw, "openclaw");
                assert_eq!(instances[0].backend, SandboxKind::Lima);
                assert_eq!(instances[0].ports.len(), 1);
                assert_eq!(instances[0].ports[0].host, 3000);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn launch_state_serializes_with_type_tag() {
        // GUI consumes LaunchState as JSON; verify the tagged-union
        // shape stays stable.
        let s = serde_json::to_string(&LaunchState::FirstRun).unwrap();
        assert_eq!(s, r#"{"type":"first_run"}"#);
        let s = serde_json::to_string(&LaunchState::NotInstalled).unwrap();
        assert_eq!(s, r#"{"type":"not_installed"}"#);
        // Ready serialises with the instances array embedded.
        let s = serde_json::to_string(&LaunchState::Ready { instances: vec![] }).unwrap();
        assert_eq!(s, r#"{"type":"ready","instances":[]}"#);
    }
}
