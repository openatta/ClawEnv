//! Bundle manifest file layout and I/O helpers.
//!
//! A bundle manifest is a small TOML file named `clawenv-bundle.toml` placed
//! at the **root** of the tar archive. It identifies exactly what's inside
//! the bundle so import-time checks can reject incompatible / malformed
//! bundles without blindly starting a VM / copying node_modules around.
//!
//! Schema discipline: `schema_version` is checked on import — unknown
//! versions bail. Adding new *optional* fields is fine within the same
//! schema; removing/renaming fields needs a schema bump.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The filename placed at the tarball root. Callers write this to the
/// tar source directory; readers extract just this entry to inspect it
/// without touching the rest of the archive.
pub const MANIFEST_FILENAME: &str = "clawenv-bundle.toml";

/// Current manifest schema version. Bump when the shape changes
/// incompatibly (e.g. rename a required field); older bundles will then be
/// rejected at import with a clear message pointing the user at a version
/// upgrade or manual migration.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleManifest {
    /// Monotonic schema marker. Import bails if the reader doesn't
    /// understand the version.
    pub schema_version: u32,

    /// clawenv release that produced this bundle (e.g. "0.2.5"). Informational.
    pub clawenv_version: String,

    /// ISO-8601 UTC timestamp.
    pub created_at: String,

    /// Claw product id from the registry ("openclaw", "hermes", …). This is
    /// the authoritative value the import side uses for `InstanceConfig`;
    /// the old "probe each cli_binary" loop is gone.
    pub claw_type: String,

    /// Version string from the claw product, as reported by
    /// `desc.version_check_cmd()`. Not parsed — purely informational so
    /// `list` / UI can show "Hermes v0.10.0" without re-running a VM probe.
    #[serde(default)]
    pub claw_version: String,

    /// Which sandbox backend produced the bundle. Encoded as kebab-case to
    /// match `SandboxType` serde representation (see `SandboxType::as_wire_str`).
    /// Readers use this to pick the right importer (Lima vs Podman vs WSL
    /// vs Native).
    pub sandbox_type: String,

    /// OS + arch of the producing machine ("darwin-aarch64",
    /// "windows-aarch64", "linux-x86_64"). Informational — cross-arch
    /// import isn't blocked by this field, just flagged to the user.
    pub source_platform: String,
}

impl BundleManifest {
    /// Serialise to TOML and write to `dir/clawenv-bundle.toml`. `dir` is
    /// the tar source directory (the same `-C <dir>` you pass to tar); the
    /// manifest ends up as `clawenv-bundle.toml` at archive root.
    pub fn write_to_dir(&self, dir: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)
            .map_err(|e| anyhow!("manifest serialize: {e}"))?;
        let path = dir.join(MANIFEST_FILENAME);
        std::fs::write(&path, body)
            .map_err(|e| anyhow!("write {}: {e}", path.display()))?;
        Ok(())
    }

    /// Parse a manifest from its TOML text representation.
    pub fn parse(text: &str) -> Result<Self> {
        let m: BundleManifest = toml::from_str(text)
            .map_err(|e| anyhow!("manifest parse: {e}"))?;
        Ok(m)
    }

    /// Extract the manifest from a `.tar.gz` bundle WITHOUT unpacking the
    /// whole archive: streams `tar -O -xzf <bundle> clawenv-bundle.toml`.
    /// The manifest is always small (~1 KB), so reading it fully into
    /// memory is fine — no need for the two-step extract-to-disk dance
    /// that `extract_inner_payload` uses.
    pub async fn peek_from_tarball(bundle: &Path) -> Result<Self> {
        let out = tokio::process::Command::new("tar")
            .args([
                "-xzf",
                &bundle.to_string_lossy(),
                "-O", // write extracted contents to stdout
                MANIFEST_FILENAME,
            ])
            .output()
            .await
            .map_err(|e| anyhow!("spawn tar for manifest peek: {e}"))?;

        // Missing-manifest is the dominant failure: v0.2.6+ requires it, and
        // pre-v0.2.6 bundles don't have one. The tar stderr for that case
        // says basically "Not found in archive" and is not useful to the
        // user, so keep the message to the actionable point. If the user
        // wants the raw details, tracing is on.
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::debug!("tar peek stderr for {}: {}", bundle.display(), stderr.trim());
            anyhow::bail!(
                "Bundle {} has no clawenv-bundle.toml manifest. Bundles produced \
                 by clawenv older than v0.2.6 (or non-clawenv archives) are not \
                 importable — re-export from the source with a current build.",
                bundle.display()
            );
        }
        let text = String::from_utf8_lossy(&out.stdout);
        if text.trim().is_empty() {
            anyhow::bail!(
                "Bundle {} has an empty manifest — the archive is malformed.",
                bundle.display()
            );
        }
        let m = Self::parse(&text)?;
        if m.schema_version > SCHEMA_VERSION {
            anyhow::bail!(
                "Bundle manifest schema_version {} is newer than this clawenv \
                 supports (max {}). Upgrade clawenv to import this bundle.",
                m.schema_version, SCHEMA_VERSION
            );
        }
        Ok(m)
    }

    /// Inner payload filename used by Podman/WSL bundles. The export side
    /// wraps the raw `podman save` / `wsl --export` tar inside an outer
    /// tar.gz along with the manifest; the import side reads this filename
    /// back out. Keeping it a named constant makes the two sides' contract
    /// explicit.
    pub const INNER_PAYLOAD_FILENAME: &'static str = "payload.tar";

    /// Wrap an already-produced inner archive (raw `podman save` or
    /// `wsl --export` output) inside an outer `.tar.gz` containing:
    ///   - `clawenv-bundle.toml`
    ///   - `payload.tar`
    ///
    /// All filesystem ops go through `tokio::fs` because the inner tar can
    /// be GB-scale (a full WSL distro export is routinely 2–4 GB) — a
    /// blocking `std::fs::rename/copy` on the tokio runtime would stall
    /// every other task on the same worker thread for the duration.
    pub async fn wrap_with_inner_tar(
        &self,
        inner_tar: &Path,
        out_path: &Path,
    ) -> Result<()> {
        let parent = out_path.parent()
            .ok_or_else(|| anyhow!("output path has no parent: {}", out_path.display()))?;
        let work = parent.join(format!(".clawenv-bundle-work-{}", std::process::id()));
        // Leftover from a prior crashed run; remove before re-creating.
        let _ = tokio::fs::remove_dir_all(&work).await;
        tokio::fs::create_dir_all(&work).await
            .map_err(|e| anyhow!("mkdir work dir {}: {e}", work.display()))?;

        // RAII-style cleanup: any early return removes the work dir so we
        // don't leak partial state next to the user's Downloads folder.
        struct WorkDirGuard(std::path::PathBuf);
        impl Drop for WorkDirGuard {
            fn drop(&mut self) {
                // Synchronous on drop — we're intentionally not awaiting here.
                // Work dir is small (manifest + possibly a moved payload), so
                // the sync remove is fine. The GB-scale payload has been
                // renamed/copied OUT by the time we drop.
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _guard = WorkDirGuard(work.clone());

        self.write_to_dir(&work)?;

        let payload_dst = work.join(Self::INNER_PAYLOAD_FILENAME);
        // Try rename first for the zero-copy fast path (same filesystem).
        // Falls back to async copy + remove if rename hits EXDEV — that's
        // the expensive case but still doesn't block the runtime.
        if tokio::fs::rename(inner_tar, &payload_dst).await.is_err() {
            tokio::fs::copy(inner_tar, &payload_dst).await
                .map_err(|e| anyhow!("copy inner tar → work dir: {e}"))?;
            let _ = tokio::fs::remove_file(inner_tar).await;
        }

        let status = tokio::process::Command::new("tar")
            .args([
                "czf",
                &out_path.to_string_lossy(),
                "-C", &work.to_string_lossy(),
                MANIFEST_FILENAME,
                Self::INNER_PAYLOAD_FILENAME,
            ])
            .status()
            .await
            .map_err(|e| anyhow!("spawn tar (wrap): {e}"))?;

        if !status.success() {
            anyhow::bail!("tar wrap exited with status {:?}", status.code());
        }
        Ok(())
    }

    /// Extract the wrapped inner payload (`payload.tar`) to `dst` so the
    /// Podman/WSL importer can hand it to `podman load` / `wsl --import`.
    ///
    /// This used to stream `tar -O ...` through stdout and `std::fs::write`
    /// the bytes — that would buffer the entire payload (can be multiple
    /// GB) into RAM and block the runtime. Now we tar-extract directly to
    /// a sibling path and `tokio::fs::rename` into place, so disk-to-disk
    /// only, no memory spike, and no blocking IO on the async executor.
    pub async fn extract_inner_payload(bundle: &Path, dst: &Path) -> Result<()> {
        let dst_parent = dst.parent().ok_or_else(|| anyhow!("dst has no parent"))?;
        tokio::fs::create_dir_all(dst_parent).await
            .map_err(|e| anyhow!("mkdir dst parent: {e}"))?;

        // Extract `payload.tar` into its own scratch dir, then move it to
        // the requested dst. Using a scratch dir (rather than extracting
        // straight into dst_parent) lets us control the filename even if
        // dst_parent already has other files, and keeps the rename atomic.
        let scratch = dst_parent.join(format!(".clawenv-extract-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&scratch).await;
        tokio::fs::create_dir_all(&scratch).await
            .map_err(|e| anyhow!("mkdir scratch dir: {e}"))?;

        let out = tokio::process::Command::new("tar")
            .args([
                "-xzf", &bundle.to_string_lossy(),
                "-C", &scratch.to_string_lossy(),
                Self::INNER_PAYLOAD_FILENAME,
            ])
            .output()
            .await
            .map_err(|e| {
                let _ = std::fs::remove_dir_all(&scratch);
                anyhow!("spawn tar (extract-inner): {e}")
            })?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let _ = tokio::fs::remove_dir_all(&scratch).await;
            tracing::debug!("tar extract-inner stderr: {}", stderr.trim());
            anyhow::bail!(
                "Bundle {} is missing the expected payload.tar — either it \
                 wasn't produced by clawenv v0.2.6+, or the archive is corrupt.",
                bundle.display()
            );
        }

        let extracted = scratch.join(Self::INNER_PAYLOAD_FILENAME);
        tokio::fs::rename(&extracted, dst).await
            .map_err(|e| anyhow!("move extracted payload → {}: {e}", dst.display()))?;
        let _ = tokio::fs::remove_dir_all(&scratch).await;
        Ok(())
    }

    /// Convenience constructor — fills in `schema_version`, `created_at`
    /// (now, UTC RFC-3339), `clawenv_version` (from Cargo pkg) and
    /// `source_platform` from the runtime.
    pub fn build(claw_type: &str, claw_version: &str, sandbox_type: &str) -> Self {
        let created_at = chrono::Utc::now().to_rfc3339();
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        Self {
            schema_version: SCHEMA_VERSION,
            clawenv_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at,
            claw_type: claw_type.to_string(),
            claw_version: claw_version.to_string(),
            sandbox_type: sandbox_type.to_string(),
            source_platform: platform,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> BundleManifest {
        BundleManifest {
            schema_version: SCHEMA_VERSION,
            clawenv_version: "0.2.6".into(),
            created_at: "2026-04-18T15:00:00+00:00".into(),
            claw_type: "hermes".into(),
            claw_version: "v0.10.0".into(),
            sandbox_type: "lima-alpine".into(),
            source_platform: "darwin-aarch64".into(),
        }
    }

    #[test]
    fn toml_roundtrip() {
        let m = sample();
        let body = toml::to_string_pretty(&m).unwrap();
        let parsed = BundleManifest::parse(&body).unwrap();
        assert_eq!(m, parsed);
    }

    #[test]
    fn write_to_dir_creates_file() {
        let tmp = TempDir::new().unwrap();
        let m = sample();
        m.write_to_dir(tmp.path()).unwrap();
        let path = tmp.path().join(MANIFEST_FILENAME);
        assert!(path.exists());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("claw_type = \"hermes\""));
    }

    #[test]
    fn parse_rejects_garbage() {
        let r = BundleManifest::parse("not valid toml {{{");
        assert!(r.is_err());
    }

    #[test]
    fn build_fills_defaults() {
        let m = BundleManifest::build("openclaw", "1.2.3", "native");
        assert_eq!(m.schema_version, SCHEMA_VERSION);
        assert!(!m.created_at.is_empty());
        assert!(!m.source_platform.is_empty());
        // clawenv_version comes from CARGO_PKG_VERSION — must be a non-empty
        // semver-ish string.
        assert!(!m.clawenv_version.is_empty());
    }

    /// End-to-end: write a manifest into a tar, then peek it back with the
    /// same code path the import side uses. Guards against the common
    /// "works on my disk, breaks through tar" class of regression.
    #[tokio::test]
    async fn peek_from_tarball_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let m = sample();
        m.write_to_dir(&src).unwrap();
        // Throw in another file so the archive isn't trivial.
        std::fs::write(src.join("dummy.txt"), "hi").unwrap();

        let bundle = tmp.path().join("test.tar.gz");
        let status = tokio::process::Command::new("tar")
            .args([
                "czf",
                &bundle.to_string_lossy(),
                "-C",
                &src.to_string_lossy(),
                MANIFEST_FILENAME,
                "dummy.txt",
            ])
            .status()
            .await
            .unwrap();
        assert!(status.success());

        let read = BundleManifest::peek_from_tarball(&bundle).await.unwrap();
        assert_eq!(read, m);
    }

    /// Round-trip the wrap/unwrap helpers: write a sentinel inner tar, wrap
    /// it with a manifest, peek the manifest, then extract the inner back
    /// out and verify its bytes match what we wrote in. This is the exact
    /// flow Podman/WSL export + import will use.
    #[tokio::test]
    async fn wrap_and_extract_inner_payload() {
        let tmp = TempDir::new().unwrap();
        let inner_src = tmp.path().join("inner-src.tar");
        let inner_bytes = b"This is a fake podman save output";
        std::fs::write(&inner_src, inner_bytes).unwrap();

        let bundle = tmp.path().join("wrapped.tar.gz");
        let m = sample();
        m.wrap_with_inner_tar(&inner_src, &bundle).await.unwrap();

        // Manifest must be peekable from the wrapped bundle.
        let peeked = BundleManifest::peek_from_tarball(&bundle).await.unwrap();
        assert_eq!(peeked, m);

        // Inner payload must round-trip byte-for-byte.
        let extracted = tmp.path().join("extracted.tar");
        BundleManifest::extract_inner_payload(&bundle, &extracted).await.unwrap();
        let got = std::fs::read(&extracted).unwrap();
        assert_eq!(got, inner_bytes);
    }

    #[tokio::test]
    async fn peek_from_tarball_bails_on_missing_manifest() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("only.txt"), "no manifest here").unwrap();

        let bundle = tmp.path().join("no-manifest.tar.gz");
        tokio::process::Command::new("tar")
            .args([
                "czf",
                &bundle.to_string_lossy(),
                "-C",
                &src.to_string_lossy(),
                "only.txt",
            ])
            .status()
            .await
            .unwrap();

        let r = BundleManifest::peek_from_tarball(&bundle).await;
        assert!(r.is_err(), "peek should fail when manifest is absent");
        let msg = format!("{}", r.unwrap_err());
        assert!(msg.contains("clawenv-bundle.toml"));
    }

    /// Guard against silent acceptance of a future-schema bundle. If
    /// `SCHEMA_VERSION` ever bumps to 2, the same producer will stamp "2"
    /// into the manifest, and this test then protects future-us from a
    /// reader that accidentally loses the version-range check.
    #[tokio::test]
    async fn peek_bails_on_newer_schema() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        let mut future = sample();
        future.schema_version = SCHEMA_VERSION + 7; // clearly "newer than us"
        future.write_to_dir(&src).unwrap();

        let bundle = tmp.path().join("future.tar.gz");
        tokio::process::Command::new("tar")
            .args(["czf", &bundle.to_string_lossy(),
                   "-C", &src.to_string_lossy(),
                   MANIFEST_FILENAME])
            .status().await.unwrap();

        let r = BundleManifest::peek_from_tarball(&bundle).await;
        assert!(r.is_err(), "peek should bail on newer schema_version");
        let msg = format!("{}", r.unwrap_err());
        // Message must mention the version numbers so the user can see what
        // to upgrade. Don't just say "unsupported" and leave them guessing.
        assert!(msg.contains(&(SCHEMA_VERSION + 7).to_string()),
                "error should mention the rejected schema_version, got: {msg}");
        assert!(msg.contains("Upgrade clawenv"),
                "error should point user at the upgrade action, got: {msg}");
    }

    /// Older/equal schema_version always succeeds. This is the forward-
    /// compatibility half of the contract: v1 readers must keep reading
    /// v1 bundles forever, even after we ship v2+ readers. When we do bump
    /// schema_version, the remediation is to add a version-dispatch
    /// deserializer — see docs/18-bundle-format.md "Schema 演进规则".
    #[tokio::test]
    async fn peek_accepts_current_and_older_schemas() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        let m = sample(); // schema_version == SCHEMA_VERSION (current)
        m.write_to_dir(&src).unwrap();

        let bundle = tmp.path().join("current.tar.gz");
        tokio::process::Command::new("tar")
            .args(["czf", &bundle.to_string_lossy(),
                   "-C", &src.to_string_lossy(),
                   MANIFEST_FILENAME])
            .status().await.unwrap();

        let r = BundleManifest::peek_from_tarball(&bundle).await;
        assert!(r.is_ok(), "peek should accept current schema_version: {r:?}");
    }
}
