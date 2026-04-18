//! Podman bundle roundtrip — requires a real `podman` on PATH.
//!
//! Gated behind `#[ignore]` because GitHub Actions hosted runners don't
//! ship podman by default; CI explicitly invokes `cargo test -- --ignored
//! podman_roundtrip` after `apt-get install -y podman`. Local dev boxes
//! can run it whenever they have podman installed.
//!
//! What this test kills off: the "wrap_with_inner_tar helper works in
//! unit tests but might misbehave on a real 10-MB `podman save` image"
//! failure mode. Unit tests use a synthetic 32-byte payload, which
//! doesn't exercise the rename-across-filesystem path or any image-tar
//! structure assumptions (`podman save` output isn't a filesystem tar).
//!
//! What this test does NOT cover: WSL roundtrip (can't on Linux/macOS
//! CI) and Native install-from-bundle's post-extract gateway start
//! (needs a real claw binary).

use clawenv_core::export::BundleManifest;
use std::path::Path;
use tempfile::TempDir;
use tokio::process::Command;

/// Cheap check: is `podman` on PATH and responsive? We don't want the
/// test to hang CI if something's half-broken, so give it a generous
/// 30s budget on just `podman --version`.
async fn podman_available() -> bool {
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        Command::new("podman").arg("--version").output(),
    )
    .await;
    matches!(res, Ok(Ok(o)) if o.status.success())
}

/// Pick the smallest image that reliably exists on Docker Hub's registry.
/// `alpine:3` is ~3 MB compressed and the Podman team uses it for their
/// own smoke tests.
const TEST_IMAGE: &str = "docker.io/library/alpine:3";

async fn podman(args: &[&str]) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    let out = Command::new("podman").args(args).output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("podman {} failed: {stderr}", args.join(" ")).into());
    }
    Ok(out)
}

#[tokio::test]
#[ignore] // requires podman — run with `cargo test -- --ignored`
async fn podman_save_wrap_unwrap_load_roundtrip() {
    if !podman_available().await {
        eprintln!("SKIP: podman not available on PATH");
        return;
    }

    // 1. Pull the tiny alpine test image. Safe to re-run — podman caches.
    podman(&["pull", "--quiet", TEST_IMAGE])
        .await
        .expect("podman pull alpine:3");

    let tmp = TempDir::new().unwrap();
    let inner_tar = tmp.path().join("podman-save.tar");
    let bundle = tmp.path().join("wrapped.tar.gz");
    let extracted = tmp.path().join("extracted.tar");

    // 2. `podman save` — produces a real container-image tar (NOT a
    // filesystem tar). This is the format our wrap helper has to handle.
    podman(&["save", "-o", inner_tar.to_str().unwrap(), TEST_IMAGE])
        .await
        .expect("podman save alpine:3");

    // Record the inner size so we can assert the wrap actually
    // round-trips the bytes and didn't silently create an empty file.
    let inner_size = tokio::fs::metadata(&inner_tar).await.unwrap().len();
    assert!(inner_size > 1000, "podman save output suspiciously small: {inner_size} bytes");

    // 3. Wrap with a Podman-flavored manifest.
    let manifest = BundleManifest::build(
        "hermes",      // fake claw_type — importer doesn't run here
        "v0.0.0-test",
        "podman-alpine",
    );
    manifest
        .wrap_with_inner_tar(&inner_tar, &bundle)
        .await
        .expect("wrap_with_inner_tar");

    // After wrap, the inner tar was renamed/copied into the work dir
    // and removed; the outer tar.gz should exist and be > 1 KB.
    assert!(bundle.exists(), "wrap did not produce {}", bundle.display());
    let bundle_size = tokio::fs::metadata(&bundle).await.unwrap().len();
    assert!(bundle_size > 1000, "wrapped bundle too small: {bundle_size}");

    // 4. Peek the manifest back out of the wrapped bundle.
    let peeked = BundleManifest::peek_from_tarball(&bundle)
        .await
        .expect("peek manifest");
    assert_eq!(peeked.claw_type, "hermes");
    assert_eq!(peeked.sandbox_type, "podman-alpine");
    assert_eq!(peeked.schema_version, 1);

    // 5. Extract the inner payload. If the wrap helper corrupted bytes
    // this is where it surfaces — `podman load` would refuse a mangled
    // image tar with a cryptic error.
    BundleManifest::extract_inner_payload(&bundle, &extracted)
        .await
        .expect("extract_inner_payload");
    let extracted_size = tokio::fs::metadata(&extracted).await.unwrap().len();
    assert_eq!(
        extracted_size, inner_size,
        "extracted payload ({extracted_size} bytes) != original save ({inner_size} bytes)"
    );

    // 6. Load the unwrapped tar back into podman. This is the full
    // lifecycle assertion — a wrap that corrupts the image would fail
    // here even if file sizes matched.
    let load_out = podman(&["load", "-i", extracted.to_str().unwrap()])
        .await
        .expect("podman load extracted payload");
    let msg = String::from_utf8_lossy(&load_out.stdout);
    assert!(
        msg.contains("alpine") || msg.contains("Loaded image"),
        "podman load output unexpected: {msg}"
    );

    // 7. Best-effort cleanup. Test is allowed to leak the alpine image on
    // the host — that's one layer of ~3 MB, negligible, and keeping it
    // around actually speeds up re-runs during development.
    let _ = tokio::fs::remove_file(&extracted).await;
    let _ = tokio::fs::remove_file(&bundle).await;
    let _ = pathsafe_remove(&inner_tar).await;
}

/// Tolerant remove_file: ignore ENOENT (the inner was renamed into the
/// wrap work dir and cleaned up there, so the original path may not
/// exist anymore — that's expected). Anything else, log and move on.
async fn pathsafe_remove(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            eprintln!("cleanup: remove_file {}: {e}", path.display());
            Ok(())
        }
    }
}
