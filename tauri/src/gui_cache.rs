//! GUI-private JSON cache at `~/.clawenv/gui-cache.json`.
//!
//! Only stores transient UI optimisations — currently the latest-version
//! check result per instance, so the tray's "update available" badge
//! doesn't have to re-hit npm on every Tauri start. v2 `InstanceConfig`
//! intentionally doesn't carry these GUI-only fields.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiCache {
    pub latest_versions: HashMap<String, LatestVersion>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LatestVersion {
    pub latest: String,
    pub checked_at: String,
}

fn cache_path() -> PathBuf {
    clawops_core::paths::clawenv_root().join("gui-cache.json")
}

pub fn load() -> GuiCache {
    match std::fs::read_to_string(cache_path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => GuiCache::default(),
    }
}

pub fn save(cache: &GuiCache) -> std::io::Result<()> {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = serde_json::to_string_pretty(cache).unwrap_or_else(|_| "{}".into());
    std::fs::write(path, s)
}

pub fn record_latest(instance: &str, latest: &str) {
    let mut c = load();
    c.latest_versions.insert(instance.into(), LatestVersion {
        latest: latest.into(),
        checked_at: chrono::Utc::now().to_rfc3339(),
    });
    let _ = save(&c);
}
