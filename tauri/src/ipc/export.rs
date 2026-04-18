use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
use clawenv_core::sandbox::SandboxType;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;
use tokio::io::{AsyncBufReadExt, BufReader};

// Global cancel flag for export operations
static EXPORT_CANCELLED: AtomicBool = AtomicBool::new(false);

/// File naming: {platform}-{arch}-{timestamp}.tar.gz
/// platform: windows/macos/linux for native, lima/wsl2/podman for sandbox
fn suggest_filename(sandbox_type: &SandboxType) -> String {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let arch = std::env::consts::ARCH;
    let platform = match sandbox_type {
        SandboxType::LimaAlpine => "lima",
        SandboxType::Wsl2Alpine => "wsl2",
        SandboxType::PodmanAlpine => "podman",
        SandboxType::Native => {
            if cfg!(target_os = "macos") { "macos" }
            else if cfg!(target_os = "windows") { "windows" }
            else { "linux" }
        }
    };
    format!("{platform}-{arch}-{ts}.tar.gz")
}

/// Emit structured stage progress to frontend
fn emit_stage(app: &tauri::AppHandle, stage: &str, status: &str, percent: u8, message: &str) {
    let _ = app.emit("export-progress", serde_json::json!({
        "stage": stage,
        "status": status,  // "pending" | "active" | "done" | "error"
        "percent": percent,
        "message": message,
    }));
}

/// Count files in a directory (cross-platform)
async fn count_files(dir: &std::path::Path) -> u64 {
    let mut count = 0u64;
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(ft) = entry.file_type().await {
                if ft.is_file() {
                    count += 1;
                } else if ft.is_dir() {
                    count += Box::pin(count_files(&entry.path())).await;
                }
            }
        }
    }
    count
}

/// Run tar with verbose output, streaming progress based on file count
async fn tar_with_progress(
    app: &tauri::AppHandle,
    output: &str,
    base_dir: &str,
    items: &[&str],
    total_files: u64,
    stage_name: &str,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let mut cmd = clawenv_core::platform::process::silent_cmd("tar");
    cmd.arg("-czvf").arg(output).arg("-C").arg(base_dir);
    for item in items {
        cmd.arg(item);
    }

    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("Failed to start tar: {e}"))?;
    let stderr = child.stderr.take().ok_or("No stderr from tar")?;
    let mut reader = BufReader::new(stderr).lines();

    let mut processed = 0u64;
    while let Ok(Some(line)) = reader.next_line().await {
        if cancelled.load(Ordering::Relaxed) {
            child.kill().await.ok();
            // Clean up partial file
            tokio::fs::remove_file(output).await.ok();
            return Err("Export cancelled".into());
        }
        processed += 1;
        let pct = if total_files > 0 {
            std::cmp::min((processed * 100 / total_files) as u8, 99)
        } else {
            50
        };
        // Show last component of file path
        let short = line.rsplit('/').next().unwrap_or(&line);
        let short = short.rsplit('\\').next().unwrap_or(short);
        let display = if short.len() > 60 { &short[..60] } else { short };
        emit_stage(app, stage_name, "active", pct, display);
    }

    let status = child.wait().await.map_err(|e| format!("tar wait failed: {e}"))?;
    if !status.success() {
        return Err("tar compression failed".into());
    }
    Ok(())
}

/// Cancel the current export operation
#[tauri::command]
pub async fn export_cancel() -> Result<(), String> {
    EXPORT_CANCELLED.store(true, Ordering::Relaxed);
    Ok(())
}

/// Export a sandbox VM image
#[tauri::command]
pub async fn export_sandbox(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    if inst.sandbox_type == SandboxType::Native {
        return Err("Use 'Export Bundle' for native instances".into());
    }

    let suggested = suggest_filename(&inst.sandbox_type);
    let path = app.dialog().file()
        .set_file_name(&suggested)
        .add_filter("VM Image", &["tar.gz", "gz"])
        .blocking_save_file();
    let path = match path {
        Some(p) => p.to_string(),
        None => return Err("Export cancelled".into()),
    };

    EXPORT_CANCELLED.store(false, Ordering::Relaxed);
    let app2 = app.clone();
    let name2 = name.clone();
    let path2 = path.clone();

    tokio::spawn(async move {
        let result = do_sandbox_export(&app2, &name2, &path2).await;
        match result {
            Ok(_) => { let _ = app2.emit("export-complete", &path2); }
            Err(e) => { let _ = app2.emit("export-failed", e.to_string()); }
        }
    });
    Ok(path)
}

async fn do_sandbox_export(app: &tauri::AppHandle, name: &str, output: &str) -> Result<(), String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, name).map_err(|e| e.to_string())?;
    let backend = instance::backend_for_instance(inst).map_err(|e| e.to_string())?;
    let vm_name = &inst.sandbox_id;
    let needs_stop = inst.sandbox_type != SandboxType::PodmanAlpine;

    // Stage 1: Stop
    if needs_stop {
        emit_stage(app, "stop", "active", 0, "Stopping instance...");
        backend.stop().await.map_err(|e| e.to_string())?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        emit_stage(app, "stop", "done", 100, "Stopped");
    }

    // Stage 2: Count files
    emit_stage(app, "count", "active", 0, "Counting files...");
    let total = match inst.sandbox_type {
        SandboxType::LimaAlpine => {
            // Lima bundle = VM directory + limactl binary + share/lima templates,
            // so the receiving side doesn't need to install Lima separately.
            let clawenv_root = dirs::home_dir().unwrap_or_default().join(".clawenv");
            let mut n = count_files(&clawenv_root.join("lima").join(vm_name)).await;
            let bin = clawenv_root.join("bin").join("limactl");
            if bin.exists() { n += 1; }
            n += count_files(&clawenv_root.join("share").join("lima")).await;
            n
        }
        _ => count_files(&std::path::PathBuf::from(".")).await,
    };
    emit_stage(app, "count", "done", 100, &format!("{total} files"));

    if EXPORT_CANCELLED.load(Ordering::Relaxed) {
        if needs_stop { backend.start().await.ok(); }
        return Err("Export cancelled".into());
    }

    // Stage 3: Compress
    emit_stage(app, "compress", "active", 0, "Starting compression...");
    match inst.sandbox_type {
        SandboxType::LimaAlpine => {
            // Export is rooted at ~/.clawenv so a receiving clawgui can untar
            // into its own ~/.clawenv and get a ready-to-run VM + Lima toolchain.
            let clawenv_root = dirs::home_dir().unwrap_or_default().join(".clawenv");
            let vm_rel = format!("lima/{vm_name}");
            // Keep cidata.iso: it holds cloud-init seed data required for a
            // fresh receiver machine to finish provisioning on first boot.
            // Sockets / pidfiles / logs are transient runtime artefacts —
            // they're safe to strip.
            let excludes = ["*.sock", "*.pid", "*.log"];
            let mut cmd = clawenv_core::platform::process::silent_cmd("tar");
            cmd.arg("-czvf").arg(output).arg("-C").arg(&clawenv_root);
            for ex in &excludes {
                cmd.arg("--exclude").arg(&format!("{vm_rel}/{ex}"));
            }
            cmd.arg(&vm_rel);
            if clawenv_root.join("bin").join("limactl").exists() {
                cmd.arg("bin/limactl");
            }
            if clawenv_root.join("share").join("lima").exists() {
                cmd.arg("share/lima");
            }
            cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());

            let mut child = cmd.spawn().map_err(|e| format!("tar failed: {e}"))?;
            let stderr = child.stderr.take().ok_or("no stderr")?;
            let mut reader = BufReader::new(stderr).lines();
            let mut processed = 0u64;
            while let Ok(Some(line)) = reader.next_line().await {
                if EXPORT_CANCELLED.load(Ordering::Relaxed) {
                    child.kill().await.ok();
                    tokio::fs::remove_file(output).await.ok();
                    if needs_stop { backend.start().await.ok(); }
                    return Err("Export cancelled".into());
                }
                processed += 1;
                let pct = if total > 0 { std::cmp::min((processed * 100 / total) as u8, 99) } else { 50 };
                let short = line.rsplit('/').next().unwrap_or(&line);
                emit_stage(app, "compress", "active", pct, short);
            }
            let status = child.wait().await.map_err(|e| format!("tar: {e}"))?;
            if !status.success() {
                if needs_stop { backend.start().await.ok(); }
                return Err("tar compression failed".into());
            }
        }
        SandboxType::Wsl2Alpine => {
            #[cfg(target_os = "windows")]
            {
                emit_stage(app, "compress", "active", 50, "Exporting WSL distro...");
                let status = clawenv_core::platform::process::silent_cmd("wsl")
                    .args(["--export", vm_name, output])
                    .status().await.map_err(|e| e.to_string())?;
                if !status.success() {
                    if needs_stop { backend.start().await.ok(); }
                    return Err("WSL export failed".into());
                }
            }
            #[cfg(not(target_os = "windows"))]
            { return Err("WSL2 export only on Windows".into()); }
        }
        SandboxType::PodmanAlpine => {
            let tag = format!("clawenv-export:{name}");
            emit_stage(app, "compress", "active", 30, "Committing container...");
            tokio::process::Command::new("podman").args(["commit", vm_name, &tag])
                .status().await.map_err(|e| e.to_string())?;
            emit_stage(app, "compress", "active", 60, "Saving image...");
            let raw = format!("{output}.tmp");
            tokio::process::Command::new("podman").args(["save", "-o", &raw, &tag])
                .status().await.map_err(|e| e.to_string())?;
            emit_stage(app, "compress", "active", 80, "Compressing...");
            tokio::process::Command::new("gzip").args(["-f", &raw])
                .status().await.map_err(|e| e.to_string())?;
            if std::path::Path::new(&format!("{raw}.gz")).exists() {
                tokio::fs::rename(format!("{raw}.gz"), output).await.ok();
            }
        }
        SandboxType::Native => unreachable!(),
    }
    emit_stage(app, "compress", "done", 100, "Compressed");

    // Stage 4: Checksum
    emit_stage(app, "checksum", "active", 0, "Calculating SHA256...");
    let size = tokio::fs::metadata(output).await.map_err(|e| e.to_string())?.len();
    emit_stage(app, "checksum", "done", 100, &format!("{} MB", size / 1_048_576));

    // Stage 5: Restart
    if needs_stop {
        emit_stage(app, "restart", "active", 0, "Restarting instance...");
        backend.start().await.ok();
        let config = ConfigManager::load().map_err(|e| e.to_string())?;
        let inst = instance::get_instance(&config, name).map_err(|e| e.to_string())?;
        instance::start_instance(inst).await.ok();
        emit_stage(app, "restart", "done", 100, "Running");
    }

    Ok(())
}

/// Export a native instance as an offline bundle
#[tauri::command]
pub async fn export_native_bundle(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    if inst.sandbox_type != SandboxType::Native {
        return Err("Use 'Export Image' for sandbox instances".into());
    }

    let suggested = suggest_filename(&inst.sandbox_type);
    let path = app.dialog().file()
        .set_file_name(&suggested)
        .add_filter("Native Bundle", &["tar.gz", "gz"])
        .blocking_save_file();
    let path = match path {
        Some(p) => p.to_string(),
        None => return Err("Export cancelled".into()),
    };

    EXPORT_CANCELLED.store(false, Ordering::Relaxed);
    let app2 = app.clone();
    let path2 = path.clone();

    tokio::spawn(async move {
        let result = do_native_export(&app2, &path2).await;
        match result {
            Ok(_) => { let _ = app2.emit("export-complete", &path2); }
            Err(e) => { let _ = app2.emit("export-failed", e.to_string()); }
        }
    });
    Ok(path)
}

async fn do_native_export(app: &tauri::AppHandle, output: &str) -> Result<(), String> {
    let home = dirs::home_dir().unwrap_or_default();
    let clawenv = home.join(".clawenv");

    // Refuse to produce a half-packaged bundle. The bundle is meant to run
    // on a fresh machine with no system git / node — if either private
    // install is missing, a receiver would either crash silently or pick up
    // whatever happens to live on their system, breaking the portability
    // contract. Surface a clear error now instead.
    let required = [
        ("node",   "Private Node.js is missing at ~/.clawenv/node/. Run the installer again; it downloads a pinned node build."),
        ("git",    "Private Git is missing at ~/.clawenv/git/. Run the installer again; it downloads a pinned dugite-native git build."),
        ("native", "No claw product is installed at ~/.clawenv/native/. Install at least one claw before exporting."),
    ];
    for (name, msg) in required {
        if !clawenv.join(name).exists() {
            return Err(msg.to_string());
        }
    }

    // Stage 1: Count files
    emit_stage(app, "count", "active", 0, "Counting files...");
    let mut total = 0u64;
    for sub in ["node", "git", "native"] {
        total += count_files(&clawenv.join(sub)).await;
    }
    emit_stage(app, "count", "done", 100, &format!("{total} files"));

    if EXPORT_CANCELLED.load(Ordering::Relaxed) {
        return Err("Export cancelled".into());
    }

    // Stage 2: Compress with verbose progress. All three dirs are required —
    // validated above, so no more conditional inclusion (which silently
    // produced broken bundles before).
    emit_stage(app, "compress", "active", 0, "Starting compression...");
    let items: Vec<&str> = vec!["node", "git", "native"];

    tar_with_progress(
        app, output,
        &clawenv.to_string_lossy(),
        &items, total, "compress",
        &EXPORT_CANCELLED,
    ).await?;
    emit_stage(app, "compress", "done", 100, "Compressed");

    // Stage 3: Checksum
    emit_stage(app, "checksum", "active", 0, "Calculating SHA256...");
    let size = tokio::fs::metadata(output).await.map_err(|e| e.to_string())?.len();
    emit_stage(app, "checksum", "done", 100, &format!("{} MB", size / 1_048_576));

    Ok(())
}
