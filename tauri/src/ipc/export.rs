//! Export IPC — thin shell over the CLI.
//!
//! Earlier versions reimplemented the tar/podman/wsl dance inside Tauri.
//! That duplicated the CLI's export logic verbatim and drifted out of sync
//! (e.g. the manifest-write step had to land in two places, cancel
//! behaviour differed subtly, and platform fixes needed double-applying).
//!
//! Now we just spawn `clawcli export` with `--json` and forward its
//! `CliEvent::Progress` events onto the Tauri `export-progress` channel.
//! Per CLAUDE.md铁律 8: "CLI 是核心，GUI 是薄壳" — this file is the薄壳.
//!
//! Cancel: `export_cancel` kills the child process via the shared
//! `CURRENT_CHILD_PID` slot. kill_on_drop is also on, so if the Tauri
//! process dies the child tar cleans up too.

use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
use clawenv_core::sandbox::SandboxType;
use std::sync::atomic::{AtomicU32, Ordering};
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;

use crate::cli_bridge::{run_cli_streaming, CliEvent};

/// PID of the currently-running export CLI child, or 0 if idle. Set when a
/// new export starts; cleared when it finishes. `export_cancel` reads this
/// to send a SIGTERM/TerminateProcess. AtomicU32 is sufficient because we
/// only run one export at a time (the filesystem dialog is modal).
static CURRENT_CHILD_PID: AtomicU32 = AtomicU32::new(0);

/// File naming: {sandbox}-{arch}-{claw_type}-{timestamp}.tar.gz
fn suggest_filename(sandbox_type: &SandboxType, claw_type: &str) -> String {
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
    format!("{platform}-{arch}-{claw_type}-{ts}.tar.gz")
}

/// Forward a single CLI event onto the frontend's `export-progress`
/// channel. The frontend's UI state machine keys off the `stage` field.
fn emit_progress(app: &tauri::AppHandle, event: &CliEvent) {
    let payload = match event {
        CliEvent::Progress { stage, percent, message } => {
            serde_json::json!({
                "stage": stage,
                "status": if *percent >= 100 { "done" } else { "active" },
                "percent": *percent,
                "message": message,
            })
        }
        CliEvent::Info { message } => {
            serde_json::json!({
                "stage": "info",
                "status": "active",
                "percent": 0,
                "message": message,
            })
        }
        CliEvent::Complete { message } => {
            serde_json::json!({
                "stage": "complete",
                "status": "done",
                "percent": 100,
                "message": message,
            })
        }
        CliEvent::Error { message, .. } => {
            serde_json::json!({
                "stage": "error",
                "status": "error",
                "percent": 100,
                "message": message,
            })
        }
        CliEvent::Data { .. } => return, // export doesn't emit Data; ignore
    };
    let _ = app.emit("export-progress", payload);
}

/// Cancel the current export operation by killing the CLI child process.
///
/// On Unix we send SIGTERM so the CLI has a chance to restart the backend
/// gateway before exiting (its RAII guards handle cleanup). On Windows we
/// `taskkill /T` to also reap any grandchildren (tar.exe, wsl.exe, etc.)
/// that the CLI spawned. The CLI's `kill_on_drop(true)` belt-and-braces
/// ensures partial tarballs don't leak.
#[tauri::command]
pub async fn export_cancel() -> Result<(), String> {
    let pid = CURRENT_CHILD_PID.load(Ordering::Relaxed);
    if pid == 0 {
        return Ok(()); // no export in flight, silent noop
    }

    #[cfg(unix)]
    {
        // kill(1) is universally available on unix — avoids adding libc
        // as a dep just to do one syscall. Fire-and-forget; we don't wait
        // on the kill process itself.
        let _ = tokio::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().await;
    }
    #[cfg(windows)]
    {
        // /T reaps the whole process tree; /F forces without asking the
        // child nicely. silent_cmd prevents a flashing console window.
        let _ = clawenv_core::platform::process::silent_cmd("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status().await;
    }
    Ok(())
}

/// Shared driver: spawns `clawcli export <name> --output <path>`, forwards
/// events, returns () on success.
///
/// Stores the child PID in `CURRENT_CHILD_PID` via `on_spawn` so
/// `export_cancel` can reach it. Resets the slot to 0 on completion so a
/// stale cancel (user clicks X after export already finished) is a no-op.
async fn run_export_cli(app: &tauri::AppHandle, name: &str, output: &str) -> Result<(), String> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<CliEvent>(64);

    let app2 = app.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            emit_progress(&app2, &event);
        }
    });

    let args: &[&str] = &["export", name, "--output", output];
    let result = run_cli_streaming(args, tx, |pid| {
        CURRENT_CHILD_PID.store(pid, Ordering::Relaxed);
    }).await;

    drop(forwarder); // rx was moved; tx dropped in run_cli_streaming; join.

    CURRENT_CHILD_PID.store(0, Ordering::Relaxed);

    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Export a sandbox VM image.
#[tauri::command]
pub async fn export_sandbox(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    if inst.sandbox_type == SandboxType::Native {
        return Err("Use 'Export Bundle' for native instances".into());
    }

    let suggested = suggest_filename(&inst.sandbox_type, &inst.claw_type);
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

    tokio::spawn(async move {
        match run_export_cli(&app2, &name2, &path2).await {
            Ok(()) => { let _ = app2.emit("export-complete", &path2); }
            Err(e) => { let _ = app2.emit("export-failed", e); }
        }
    });
    Ok(path)
}

/// Export a native instance as an offline bundle.
#[tauri::command]
pub async fn export_native_bundle(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &name).map_err(|e| e.to_string())?;
    if inst.sandbox_type != SandboxType::Native {
        return Err("Use 'Export Image' for sandbox instances".into());
    }

    let suggested = suggest_filename(&inst.sandbox_type, &inst.claw_type);
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
        match run_export_cli(&app2, &name2, &path2).await {
            Ok(()) => { let _ = app2.emit("export-complete", &path2); }
            Err(e) => { let _ = app2.emit("export-failed", e); }
        }
    });
    Ok(path)
}
