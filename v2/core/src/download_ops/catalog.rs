//! TOML-backed artifact catalog.

use serde::Deserialize;

use super::types::{ArtifactSpec, PlatformKey};

#[derive(Debug, Clone, Default)]
pub struct DownloadCatalog {
    artifacts: Vec<ArtifactSpec>,
}

#[derive(Deserialize)]
struct CatalogFile {
    #[serde(default, rename = "artifact")]
    artifacts: Vec<ArtifactSpec>,
}

impl DownloadCatalog {
    pub fn empty() -> Self { Self { artifacts: Vec::new() } }

    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        let file: CatalogFile = toml::from_str(s)?;
        Ok(Self { artifacts: file.artifacts })
    }

    /// Builtin catalog embedded at compile time from `v2/assets/download-catalog.toml`.
    pub fn builtin() -> Self {
        Self::from_toml_str(include_str!("../../../assets/download-catalog.toml"))
            .expect("embedded catalog must parse — CI should catch drift")
    }

    pub fn artifacts(&self) -> &[ArtifactSpec] { &self.artifacts }

    /// Find by (name, optional version). If version is None, returns the
    /// first matching artifact for the current platform (catalog order =
    /// preference order).
    pub fn find(
        &self,
        name: &str,
        version: Option<&str>,
        platform: &PlatformKey,
    ) -> Option<&ArtifactSpec> {
        self.artifacts.iter().find(|a|
            a.name == name
            && a.platform == *platform
            && version.is_none_or(|v| a.version == v)
        )
    }

    /// Returns all entries matching `name` (any version / any platform).
    pub fn by_name(&self, name: &str) -> Vec<&ArtifactSpec> {
        self.artifacts.iter().filter(|a| a.name == name).collect()
    }

    /// Filter by platform.
    pub fn by_platform(&self, platform: &PlatformKey) -> Vec<&ArtifactSpec> {
        self.artifacts.iter().filter(|a| a.platform == *platform).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download_ops::types::ArtifactKind;

    const SAMPLE: &str = r#"
        [[artifact]]
        name = "node"
        version = "22.12.0"
        os = "macos"
        arch = "arm64"
        url = "https://example.com/node.tar.gz"
        kind = "tarball"

        [[artifact]]
        name = "node"
        version = "22.12.0"
        os = "linux"
        arch = "x86_64"
        url = "https://example.com/node-linux.tar.xz"
        kind = "tarball"
        sha256 = "abc"

        [[artifact]]
        name = "git"
        version = "2.45.0"
        os = "macos"
        arch = "arm64"
        url = "https://example.com/git.tar.gz"
        kind = "tarball"
    "#;

    #[test]
    fn parses_toml() {
        let c = DownloadCatalog::from_toml_str(SAMPLE).unwrap();
        assert_eq!(c.artifacts().len(), 3);
    }

    #[test]
    fn find_exact_platform() {
        let c = DownloadCatalog::from_toml_str(SAMPLE).unwrap();
        let key = PlatformKey { os: "macos".into(), arch: "arm64".into() };
        let a = c.find("node", Some("22.12.0"), &key).unwrap();
        assert_eq!(a.url, "https://example.com/node.tar.gz");
        assert!(matches!(a.kind, ArtifactKind::Tarball));
    }

    #[test]
    fn find_none_for_unknown() {
        let c = DownloadCatalog::from_toml_str(SAMPLE).unwrap();
        let key = PlatformKey { os: "macos".into(), arch: "arm64".into() };
        assert!(c.find("nonexistent", None, &key).is_none());
    }

    #[test]
    fn by_name_collects_versions() {
        let c = DownloadCatalog::from_toml_str(SAMPLE).unwrap();
        assert_eq!(c.by_name("node").len(), 2);
        assert_eq!(c.by_name("git").len(), 1);
    }

    #[test]
    fn builtin_parses() {
        let c = DownloadCatalog::builtin();
        assert!(!c.artifacts().is_empty());
    }

    /// Contract test: the embedded catalog must cover every (artifact,
    /// platform) pair that v1's mirrors.toml used to cover. If this fails,
    /// v2 can't replace v1 for that combo without a network-level regression.
    /// Update the expectations list when catalog scope legitimately changes.
    #[test]
    fn builtin_covers_all_v1_platforms() {
        let c = DownloadCatalog::builtin();
        // (name, os, arch) tuples that were downloadable in v1.
        let required: &[(&str, &str, &str)] = &[
            // node — portable runtime (6 platforms)
            ("node", "macos", "arm64"),
            ("node", "macos", "x86_64"),
            ("node", "linux", "arm64"),
            ("node", "linux", "x86_64"),
            ("node", "windows", "arm64"),
            ("node", "windows", "x86_64"),
            // git — dugite-native (unix) + MinGit (windows)
            ("git", "macos", "arm64"),
            ("git", "macos", "x86_64"),
            ("git", "linux", "arm64"),
            ("git", "linux", "x86_64"),
            ("git", "windows", "arm64"),
            ("git", "windows", "x86_64"),
            // lima — macOS-only
            ("lima", "macos", "arm64"),
            ("lima", "macos", "x86_64"),
            // alpine-rootfs — WSL2 + Podman base (linux arches)
            ("alpine-rootfs", "linux", "arm64"),
            ("alpine-rootfs", "linux", "x86_64"),
        ];
        for (name, os, arch) in required {
            let key = PlatformKey { os: (*os).into(), arch: (*arch).into() };
            assert!(
                c.find(name, None, &key).is_some(),
                "builtin catalog missing ({name}, {os}, {arch})",
            );
        }
    }

    #[test]
    fn sha256_is_optional() {
        let c = DownloadCatalog::from_toml_str(SAMPLE).unwrap();
        let key = PlatformKey { os: "linux".into(), arch: "x86_64".into() };
        assert_eq!(c.find("node", None, &key).unwrap().sha256.as_deref(), Some("abc"));
        let key_mac = PlatformKey { os: "macos".into(), arch: "arm64".into() };
        assert!(c.find("node", None, &key_mac).unwrap().sha256.is_none());
    }
}
