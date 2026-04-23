//! Instance registry — TOML file at `<clawenv_root>/v2/instances.toml`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::common::OpsError;
use crate::paths::v2_instances_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxKind {
    /// Claw runs directly on host (portable Node/Git under `~/.clawenv/`).
    Native,
    /// Lima VM (macOS).
    Lima,
    /// WSL2 distro (Windows).
    Wsl2,
    /// Podman container (Linux).
    Podman,
}

impl SandboxKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Lima => "lima",
            Self::Wsl2 => "wsl2",
            Self::Podman => "podman",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "native" => Some(Self::Native),
            "lima" => Some(Self::Lima),
            "wsl2" => Some(Self::Wsl2),
            "podman" => Some(Self::Podman),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortBinding {
    pub host: u16,
    pub guest: u16,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    /// Unique name (typed by user, e.g. "default").
    pub name: String,
    /// Which claw product (e.g. "hermes", "openclaw").
    pub claw: String,
    /// Where the claw runs.
    pub backend: SandboxKind,
    /// VM / container instance name (only meaningful for sandboxed backends).
    #[serde(default)]
    pub sandbox_instance: String,
    #[serde(default)]
    pub ports: Vec<PortBinding>,
    /// RFC3339 timestamp.
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    /// Optional user-provided notes.
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstanceFile {
    #[serde(default, rename = "instance")]
    pub instances: Vec<InstanceConfig>,
}

/// CRUD over the instance registry file.
///
/// Read/write is atomic via temp-file + rename. Concurrent v2 processes
/// should not race: v2's current CLI is single-shot; a future daemon would
/// add advisory locking.
pub struct InstanceRegistry {
    path: PathBuf,
}

impl InstanceRegistry {
    pub fn with_default_path() -> Self {
        Self { path: v2_instances_path() }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &PathBuf { &self.path }

    /// Read the registry (empty if file missing).
    pub async fn load(&self) -> Result<InstanceFile, OpsError> {
        if !self.path.exists() {
            return Ok(InstanceFile::default());
        }
        let s = fs::read_to_string(&self.path).await?;
        let parsed: InstanceFile = toml::from_str(&s)
            .map_err(|e| OpsError::parse(format!("parse {}: {e}", self.path.display())))?;
        Ok(parsed)
    }

    /// Write the registry back (creates parent dirs if needed).
    pub async fn save(&self, file: &InstanceFile) -> Result<(), OpsError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let s = toml::to_string_pretty(file)
            .map_err(|e| OpsError::parse(format!("serialize: {e}")))?;
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, s).await?;
        fs::rename(&tmp, &self.path).await?;
        Ok(())
    }

    pub async fn find(&self, name: &str) -> Result<Option<InstanceConfig>, OpsError> {
        let file = self.load().await?;
        Ok(file.instances.into_iter().find(|i| i.name == name))
    }

    /// Insert. Errors if name already exists.
    pub async fn insert(&self, inst: InstanceConfig) -> Result<(), OpsError> {
        let mut file = self.load().await?;
        if file.instances.iter().any(|i| i.name == inst.name) {
            return Err(OpsError::unsupported(
                "insert",
                format!("instance `{}` already exists", inst.name),
            ));
        }
        file.instances.push(inst);
        self.save(&file).await
    }

    /// Replace the record with matching name, or return NotFound.
    pub async fn update(&self, inst: InstanceConfig) -> Result<(), OpsError> {
        let mut file = self.load().await?;
        let idx = file.instances.iter().position(|i| i.name == inst.name)
            .ok_or_else(|| OpsError::not_found(format!("instance `{}`", inst.name)))?;
        file.instances[idx] = inst;
        self.save(&file).await
    }

    /// Remove by name. Returns the removed record, or NotFound.
    pub async fn remove(&self, name: &str) -> Result<InstanceConfig, OpsError> {
        let mut file = self.load().await?;
        let idx = file.instances.iter().position(|i| i.name == name)
            .ok_or_else(|| OpsError::not_found(format!("instance `{name}`")))?;
        let removed = file.instances.remove(idx);
        self.save(&file).await?;
        Ok(removed)
    }

    pub async fn list(&self) -> Result<Vec<InstanceConfig>, OpsError> {
        Ok(self.load().await?.instances)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample(name: &str) -> InstanceConfig {
        InstanceConfig {
            name: name.into(),
            claw: "hermes".into(),
            backend: SandboxKind::Lima,
            sandbox_instance: name.into(),
            ports: vec![PortBinding { host: 3000, guest: 3000, label: "gateway".into() }],
            created_at: "2026-04-23T00:00:00Z".into(),
            updated_at: String::new(),
            note: String::new(),
        }
    }

    #[tokio::test]
    async fn empty_registry_loads_empty() {
        let tmp = TempDir::new().unwrap();
        let reg = InstanceRegistry::with_path(tmp.path().join("insts.toml"));
        let f = reg.load().await.unwrap();
        assert!(f.instances.is_empty());
    }

    #[tokio::test]
    async fn insert_list_round_trip() {
        let tmp = TempDir::new().unwrap();
        let reg = InstanceRegistry::with_path(tmp.path().join("insts.toml"));
        reg.insert(sample("default")).await.unwrap();
        reg.insert(sample("second")).await.unwrap();
        let all = reg.list().await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|i| i.name == "default"));
    }

    #[tokio::test]
    async fn insert_duplicate_errs() {
        let tmp = TempDir::new().unwrap();
        let reg = InstanceRegistry::with_path(tmp.path().join("insts.toml"));
        reg.insert(sample("a")).await.unwrap();
        let err = reg.insert(sample("a")).await.unwrap_err();
        assert!(matches!(err, OpsError::Unsupported { .. }));
    }

    #[tokio::test]
    async fn remove_then_find_returns_none() {
        let tmp = TempDir::new().unwrap();
        let reg = InstanceRegistry::with_path(tmp.path().join("insts.toml"));
        reg.insert(sample("x")).await.unwrap();
        reg.remove("x").await.unwrap();
        assert!(reg.find("x").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_missing_errs_not_found() {
        let tmp = TempDir::new().unwrap();
        let reg = InstanceRegistry::with_path(tmp.path().join("insts.toml"));
        let err = reg.remove("ghost").await.unwrap_err();
        assert!(matches!(err, OpsError::NotFound { .. }));
    }

    #[tokio::test]
    async fn update_replaces_record() {
        let tmp = TempDir::new().unwrap();
        let reg = InstanceRegistry::with_path(tmp.path().join("insts.toml"));
        reg.insert(sample("a")).await.unwrap();
        let mut modified = sample("a");
        modified.note = "hello".into();
        reg.update(modified).await.unwrap();
        let fetched = reg.find("a").await.unwrap().unwrap();
        assert_eq!(fetched.note, "hello");
    }

    #[test]
    fn sandbox_kind_roundtrip() {
        for k in [SandboxKind::Native, SandboxKind::Lima, SandboxKind::Wsl2, SandboxKind::Podman] {
            assert_eq!(SandboxKind::parse(k.as_str()), Some(k));
        }
        assert!(SandboxKind::parse("bogus").is_none());
    }
}
