use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
use clawenv_core::sandbox::SandboxType;
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;

/// Build a suggested file name for export.
/// Sandbox: {instance}-{backend}-{timestamp}.tar.gz
/// Native:  {instance}-{platform}-bundle-{timestamp}.tar.gz
fn suggest_filename(inst_name: &str, sandbox_type: &SandboxType) -> String {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    match sandbox_type {
        SandboxType::LimaAlpine => format!("{inst_name}-lima-{ts}.tar.gz"),
        SandboxType::Wsl2Alpine => format!("{inst_name}-wsl2-{ts}.tar.gz"),
        SandboxType::PodmanAlpine => format!("{inst_name}-podman-{ts}.tar.gz"),
        SandboxType::Native => {
            let platform = if cfg!(target_os = "macos") { "macos" }
                else if cfg!(target_os = "windows") { "windows" }
                else { "linux" };
            format!("{inst_name}-{platform}-bundle-{ts}.tar.gz")
        }
    }
}

fn emit_progress(app: &tauri::AppHandle, percent: u8, message: &str) {
    let _ = app.emit("export-progress", serde_json::json!({
        "percent": percent,
        "message": message,
    }));
}

/// Export a sandbox VM image. Stops VM if needed, exports, restarts.
#[tauri::command]
pub async fn export_sandbox(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;

    if inst.sandbox_type == SandboxType::Native {
        return Err("Use 'Export Bundle' for native instances".into());
    }

    let suggested = suggest_filename(&name, &inst.sandbox_type);

    // Show save dialog
    let path = app.dialog().file()
        .set_file_name(&suggested)
        .add_filter("VM Image", &["tar.gz", "gz"])
        .blocking_save_file();

    let path = match path {
        Some(p) => p.to_string(),
        None => return Err("Export cancelled".into()),
    };

    let app2 = app.clone();
    let name2 = name.clone();
    let path2 = path.clone();

    // Run export in background
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

    // Phase 1: Stop instance
    let needs_stop = inst.sandbox_type != SandboxType::PodmanAlpine;
    if needs_stop {
        emit_progress(app, 10, "Stopping instance for export...");
        backend.stop().await.map_err(|e| e.to_string())?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    // Phase 2: Export
    emit_progress(app, 20, "Exporting image (this may take several minutes)...");

    let status = match inst.sandbox_type {
        SandboxType::LimaAlpine => {
            // Lima: tar the VM directory
            let lima_dir = format!("{}/.lima", std::env::var("HOME").unwrap_or_default());
            let mut cmd = tokio::process::Command::new("tar");
            cmd.args(["-czf", output, "-C", &lima_dir,
                "--exclude", &format!("{vm_name}/*.sock"),
                "--exclude", &format!("{vm_name}/*.pid"),
                "--exclude", &format!("{vm_name}/*.log"),
                "--exclude", &format!("{vm_name}/cidata.iso"),
                vm_name]);
            cmd.status().await.map_err(|e| e.to_string())?
        }
        SandboxType::Wsl2Alpine => {
            // WSL2: wsl --export
            #[cfg(target_os = "windows")]
            {
                clawenv_core::platform::process::silent_cmd("wsl")
                    .args(["--export", vm_name, output])
                    .status().await.map_err(|e| e.to_string())?
            }
            #[cfg(not(target_os = "windows"))]
            { return Err("WSL2 export only available on Windows".into()); }
        }
        SandboxType::PodmanAlpine => {
            // Podman: commit + save
            emit_progress(app, 30, "Committing container state...");
            let tag = format!("clawenv-export:{name}");
            tokio::process::Command::new("podman")
                .args(["commit", vm_name, &tag])
                .status().await.map_err(|e| e.to_string())?;

            emit_progress(app, 50, "Saving image...");
            let raw = format!("{}.tmp", output);
            tokio::process::Command::new("podman")
                .args(["save", "-o", &raw, &tag])
                .status().await.map_err(|e| e.to_string())?;

            emit_progress(app, 70, "Compressing...");
            tokio::process::Command::new("gzip")
                .args(["-f", &raw])
                .status().await.map_err(|e| e.to_string())?;

            // gzip renames raw → raw.gz, but we want the target name
            if std::path::Path::new(&format!("{raw}.gz")).exists() {
                tokio::fs::rename(format!("{raw}.gz"), output).await.map_err(|e| e.to_string())?;
            }
            std::process::ExitStatus::default()
        }
        SandboxType::Native => unreachable!(),
    };

    if inst.sandbox_type != SandboxType::PodmanAlpine && !status.success() {
        // Restart even on failure
        if needs_stop { backend.start().await.ok(); }
        return Err("Export command failed".into());
    }

    emit_progress(app, 85, "Generating checksum...");
    // SHA256
    let file_bytes = tokio::fs::read(output).await.map_err(|e| e.to_string())?;
    let hash = sha2_hex(&file_bytes);
    let size = file_bytes.len();

    emit_progress(app, 90, "Writing manifest...");
    let manifest = format!("{}.manifest.toml", output.trim_end_matches(".tar.gz"));
    let manifest_content = format!(
        "[package]\ninstance = \"{name}\"\nplatform = \"{}\"\narch = \"{}\"\n\
         [image]\nfile = \"{}\"\nsize_bytes = {size}\nsha256 = \"{hash}\"\n\
         [clawenv]\nversion = \"0.2.0\"\n",
        std::env::consts::OS, std::env::consts::ARCH,
        std::path::Path::new(output).file_name().unwrap_or_default().to_string_lossy()
    );
    tokio::fs::write(&manifest, manifest_content).await.ok();

    // Phase 3: Restart
    if needs_stop {
        emit_progress(app, 95, "Restarting instance...");
        backend.start().await.ok();
        // Also restart services
        let config = ConfigManager::load().map_err(|e| e.to_string())?;
        let inst = instance::get_instance(&config, name).map_err(|e| e.to_string())?;
        instance::start_instance(inst).await.ok();
    }

    emit_progress(app, 100, &format!("Export complete ({} MB)", size / 1_048_576));
    Ok(())
}

fn sha2_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Export a native instance as an offline bundle.
#[tauri::command]
pub async fn export_native_bundle(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;

    if inst.sandbox_type != SandboxType::Native {
        return Err("Use 'Export Image' for sandbox instances".into());
    }

    let suggested = suggest_filename(&name, &inst.sandbox_type);

    let path = app.dialog().file()
        .set_file_name(&suggested)
        .add_filter("Native Bundle", &["tar.gz", "gz"])
        .blocking_save_file();

    let path = match path {
        Some(p) => p.to_string(),
        None => return Err("Export cancelled".into()),
    };

    let app2 = app.clone();
    let name2 = name.clone();
    let path2 = path.clone();

    tokio::spawn(async move {
        let result = do_native_export(&app2, &name2, &path2).await;
        match result {
            Ok(_) => { let _ = app2.emit("export-complete", &path2); }
            Err(e) => { let _ = app2.emit("export-failed", e.to_string()); }
        }
    });

    Ok(path)
}

async fn do_native_export(app: &tauri::AppHandle, name: &str, output: &str) -> Result<(), String> {
    let home = dirs::home_dir().unwrap_or_default();
    let install_dir = home.join(".clawenv/native").join(name);

    if !install_dir.exists() {
        return Err(format!("Native install dir not found: {}", install_dir.display()));
    }

    emit_progress(app, 10, "Preparing native bundle...");

    // Include node runtime + node_modules
    let node_dir = home.join(".clawenv/node");

    emit_progress(app, 20, "Compressing bundle (this may take several minutes)...");

    let mut cmd = tokio::process::Command::new("tar");
    cmd.arg("-czf").arg(output).arg("-C").arg(home.join(".clawenv"));

    // Include node/ if it exists (ClawEnv managed Node.js)
    if node_dir.exists() {
        cmd.arg("node");
    }
    // Include the instance directory
    cmd.arg(format!("native/{name}"));

    let status = cmd.status().await.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("tar compression failed".into());
    }

    emit_progress(app, 90, "Generating checksum...");
    let size = tokio::fs::metadata(output).await.map_err(|e| e.to_string())?.len();

    emit_progress(app, 100, &format!("Export complete ({} MB)", size / 1_048_576));
    Ok(())
}
