//! Lite-specific IPC commands — package scanning for offline install.

use serde::Serialize;

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
}

/// Scan the app's directory for compatible .tar.gz packages.
#[tauri::command]
pub async fn lite_scan_packages() -> Result<Vec<PackageInfo>, String> {
    // Get the directory where the app executable lives
    let exe_dir = std::env::current_exe()
        .map_err(|e| e.to_string())?;

    // Navigate to the app's parent directory
    #[cfg(target_os = "macos")]
    let scan_dir = exe_dir
        .parent() // MacOS/
        .and_then(|p| p.parent()) // Contents/
        .and_then(|p| p.parent()) // xxx.app
        .and_then(|p| p.parent()) // parent dir
        .unwrap_or(exe_dir.parent().unwrap_or(std::path::Path::new(".")));

    #[cfg(not(target_os = "macos"))]
    let scan_dir = exe_dir.parent().unwrap_or(std::path::Path::new("."));

    let current_platform = if cfg!(target_os = "macos") { "macos" }
        else if cfg!(target_os = "windows") { "windows" }
        else { "linux" };
    let current_backend = if cfg!(target_os = "macos") { "lima" }
        else if cfg!(target_os = "windows") { "wsl2" }
        else { "podman" };
    let current_arch = std::env::consts::ARCH;

    // Check sandbox backend availability
    let sandbox_available = match clawenv_core::sandbox::detect_backend() {
        Ok(b) => b.is_available().await.unwrap_or(false),
        Err(_) => false,
    };

    let mut packages = Vec::new();

    if let Ok(entries) = std::fs::read_dir(scan_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let filename = path.file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();

            // Only .tar.gz files
            if !filename.ends_with(".tar.gz") { continue; }

            // Parse: {platform}-{arch}-{timestamp}.tar.gz
            let parts: Vec<String> = filename.split('-').map(String::from).collect();
            if parts.len() < 3 { continue; }

            let file_platform = &parts[0];
            let file_arch = &parts[1];

            let is_native = matches!(file_platform.as_str(), "windows" | "macos" | "linux");
            let is_sandbox = matches!(file_platform.as_str(), "lima" | "wsl2" | "podman");
            if !is_native && !is_sandbox { continue; }

            // Check platform compatibility
            let platform_ok = if is_native {
                file_platform == current_platform
            } else {
                file_platform == current_backend
            };

            // Check arch compatibility
            let arch_ok = file_arch == current_arch
                || (file_arch == "arm64" && current_arch == "aarch64")
                || (file_arch == "aarch64" && current_arch == "aarch64")
                || (file_arch == "x64" && current_arch == "x86_64")
                || (file_arch == "x86_64" && current_arch == "x86_64");

            let size_mb = entry.metadata().map(|m| m.len() / 1_048_576).unwrap_or(0);

            packages.push(PackageInfo {
                path: path.to_string_lossy().to_string(),
                filename,
                platform: file_platform.clone(),
                arch: file_arch.clone(),
                is_native,
                size_mb,
                compatible: platform_ok && arch_ok,
                needs_sandbox_backend: is_sandbox,
                sandbox_backend_available: sandbox_available,
            });
        }
    }

    // Sort: compatible first, then by type (native first)
    packages.sort_by(|a, b| {
        b.compatible.cmp(&a.compatible)
            .then(a.is_native.cmp(&b.is_native).reverse())
    });

    Ok(packages)
}
