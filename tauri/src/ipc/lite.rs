//! Lite-specific IPC commands — package scanning for offline install.
//!
//! Lite picks a `.tar.gz` bundle from the same folder as the app binary
//! and installs it. Unlike the full-app import wizard, Lite has **no
//! online install path** — it's offline-first, bundle-driven. So the
//! scanner has to surface everything the UI needs to render the pick
//! step: OS/arch compatibility, native-conflict, AND the claw product
//! identity (type + version + display name). The latter comes from the
//! bundle's own manifest (`clawenv-bundle.toml`), not from the
//! filename — filenames can lie, manifests are authoritative.

use clawops_core::export::BundleManifest;
use serde::Serialize;
use std::path::Path;

use crate::claw_meta;

#[derive(Debug, Serialize)]
pub struct PackageInfo {
    pub path: String,
    pub filename: String,
    pub platform: String,
    pub arch: String,
    pub is_native: bool,
    pub size_mb: u64,
    pub compatible: bool,
    pub needs_sandbox_backend: bool,
    pub sandbox_backend_available: bool,

    // Identity fields from the bundle manifest. Empty string = manifest
    // was missing/unreadable (which also sets compatible=false since we
    // can't safely install a bundle we can't identify). `claw_display_name`
    // is resolved via the built-in claw registry at scan time so the UI
    // doesn't need a separate lookup round-trip.
    pub claw_type: String,
    pub claw_version: String,
    pub claw_display_name: String,

    /// Short reason string when `compatible=false`, for UI tooltips.
    /// Empty when compatible. Keeps the enum-of-errors logic in one
    /// place instead of scattering bool flags across the struct.
    pub reason: String,
}

/// Scan a directory for compatible .tar.gz packages.
///
/// When `scan_dir` is `None`, defaults to the folder the app binary lives
/// in — the common case on first launch. When provided, the user picked
/// a folder via `pick_import_folder`; behaviour is identical except the
/// scan target is under the caller's control. Returns both compatible
/// and incompatible bundles (UI greys out the incompatible ones) so the
/// user can see what's in the folder even when nothing is installable.
#[tauri::command]
pub async fn lite_scan_packages(scan_dir: Option<String>) -> Result<Vec<PackageInfo>, String> {
    let dir = match scan_dir {
        Some(d) => std::path::PathBuf::from(d),
        None => scan_dir_for_app()?,
    };
    let ctx = HostContext::detect().await;

    let mut packages = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let filename = path.file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            if !filename.ends_with(".tar.gz") { continue; }
            if let Some(pkg) = inspect_bundle(&path, &ctx).await {
                packages.push(pkg);
            }
        }
    }

    // Sort: compatible first, then native before sandbox for equal compatibility.
    packages.sort_by(|a, b| {
        b.compatible.cmp(&a.compatible)
            .then(a.is_native.cmp(&b.is_native).reverse())
    });
    Ok(packages)
}

/// Open a native folder-picker and return the chosen path.
/// Used by the Lite scanner's "Choose folder..." escape hatch so the
/// user can point Lite at bundles outside the app folder (USB stick,
/// network drive, Downloads). Returns `None` when the user cancels.
#[tauri::command]
pub async fn pick_import_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app.dialog().file().blocking_pick_folder();
    Ok(path.map(|p| p.to_string()))
}

struct HostContext {
    platform: &'static str,
    backend: &'static str,
    arch: &'static str,
    sandbox_available: bool,
}

impl HostContext {
    async fn detect() -> Self {
        // v2 doesn't expose a single `detect_backend()` factory — each
        // SandboxBackend impl has its own `is_available()` predicate.
        // Build the host's default and probe it directly.
        use clawops_core::sandbox_backend::{
            LimaBackend, PodmanBackend, SandboxBackend, WslBackend,
        };
        let sandbox_available = if cfg!(target_os = "macos") {
            LimaBackend::new("default".to_string())
                .is_available().await.unwrap_or(false)
        } else if cfg!(target_os = "windows") {
            WslBackend::new("default".to_string())
                .is_available().await.unwrap_or(false)
        } else {
            PodmanBackend::new("default".to_string())
                .is_available().await.unwrap_or(false)
        };
        Self {
            platform: if cfg!(target_os = "macos") { "macos" }
                else if cfg!(target_os = "windows") { "windows" }
                else { "linux" },
            backend: if cfg!(target_os = "macos") { "lima" }
                else if cfg!(target_os = "windows") { "wsl2" }
                else { "podman" },
            arch: std::env::consts::ARCH,
            sandbox_available,
        }
    }
}

fn scan_dir_for_app() -> Result<std::path::PathBuf, String> {
    let exe_dir = std::env::current_exe().map_err(|e| e.to_string())?;
    // On macOS, the exe lives inside ClawLite.app/Contents/MacOS/ — we
    // want the folder that CONTAINS the .app, i.e. four levels up.
    #[cfg(target_os = "macos")]
    let scan_dir = exe_dir
        .parent().and_then(|p| p.parent())
        .and_then(|p| p.parent()).and_then(|p| p.parent())
        .unwrap_or(exe_dir.parent().unwrap_or(std::path::Path::new(".")))
        .to_path_buf();
    #[cfg(not(target_os = "macos"))]
    let scan_dir = exe_dir.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    Ok(scan_dir)
}

/// Build a PackageInfo for a single .tar.gz path. Returns None only when
/// the file isn't shaped like a bundle (no valid filename). An unreadable
/// manifest yields an INCOMPATIBLE PackageInfo (with reason text) instead
/// of None — we want the user to see "this tar.gz is in the folder but
/// can't be used" rather than silently drop it.
async fn inspect_bundle(path: &Path, ctx: &HostContext) -> Option<PackageInfo> {
    let filename = path.file_name()?.to_string_lossy().to_string();
    let size_mb = std::fs::metadata(path).map(|m| m.len() / 1_048_576).unwrap_or(0);

    // Filename parse is still used for platform/arch display. The
    // format `{platform}-{arch}-{timestamp}.tar.gz` is what clawenv's
    // own exporter produces, but we DON'T treat filename as authoritative
    // for claw_type anymore — that's what the manifest is for.
    let stem = filename.trim_end_matches(".tar.gz");
    let parts: Vec<&str> = stem.split('-').collect();
    let (file_platform, file_arch) = if parts.len() >= 2 {
        (parts[0].to_string(), parts[1].to_string())
    } else {
        (String::new(), String::new())
    };

    let is_native = matches!(file_platform.as_str(), "windows" | "macos" | "linux");
    let is_sandbox = matches!(file_platform.as_str(), "lima" | "wsl2" | "podman");

    // Read manifest from the tarball. Source of truth for claw_type /
    // claw_version. When the manifest is missing, the bundle is either
    // ancient (pre-v0.2.6) or not ours — either way, don't attempt.
    let manifest = BundleManifest::peek_from_tarball(path).await;

    let platform_ok = if is_native { file_platform == ctx.platform }
        else if is_sandbox { file_platform == ctx.backend }
        else { false };
    let arch_ok = arch_matches(&file_arch, ctx.arch);

    let mut reason = String::new();
    let mut compatible = platform_ok && arch_ok;

    let (claw_type, claw_version) = match &manifest {
        Ok(m) => (m.claw_type.clone(), m.claw_version.clone()),
        Err(e) => {
            compatible = false;
            if reason.is_empty() {
                // Hide the long "re-export from source with a current build"
                // text from the card — a short phrase is enough; the user
                // reads the full text once they hover / the install attempt
                // lands with the real error.
                reason = format!("Invalid bundle: {}", first_line(&e.to_string()));
            }
            (String::new(), String::new())
        }
    };

    if reason.is_empty() && !platform_ok {
        reason = format!("Bundle for {}/{}, host is {}/{}",
            file_platform, file_arch, ctx.platform, ctx.arch);
    }
    if reason.is_empty() && !arch_ok {
        reason = format!("Bundle for arch {}, host is {}", file_arch, ctx.arch);
    }

    // Resolve display name from the static GUI table. Unknown claw_type
    // falls back to its raw id (capitalised) so the UI still shows
    // something sensible.
    let claw_display_name = if claw_type.is_empty() {
        String::new()
    } else {
        claw_meta::meta_for(&claw_type).display_name
    };

    Some(PackageInfo {
        path: path.to_string_lossy().to_string(),
        filename,
        platform: file_platform,
        arch: file_arch,
        is_native,
        size_mb,
        compatible,
        needs_sandbox_backend: is_sandbox,
        sandbox_backend_available: ctx.sandbox_available,
        claw_type,
        claw_version,
        claw_display_name,
        reason,
    })
}

fn arch_matches(file: &str, host: &str) -> bool {
    file == host
        || (file == "arm64" && host == "aarch64")
        || (file == "aarch64" && host == "aarch64")
        || (file == "x64" && host == "x86_64")
        || (file == "x86_64" && host == "x86_64")
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}
