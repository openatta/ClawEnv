use clawops_core::credentials;
use clawops_core::proxy::{ProxyConfig, ProxySource, ProxyTriple};
use clawops_core::wire::{SystemCheckItem as ApiCheckItem, SystemInfo as SystemCheckResponse};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

use crate::cli_bridge::{self, CliEvent};
use crate::ipc::emit::{emit_instance_changed, InstanceAction, InstanceChanged};

/// Guard against concurrent installs — only one install at a time.
static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);

/// RAII guard that resets `INSTALL_RUNNING` on drop. A previous iteration
/// set the flag on entry and cleared it at the end of the happy path — if
/// any code in the async task panicked (tokio catches panics but the
/// trailing `store(false)` still got skipped), the flag stayed `true`
/// forever, leaving the user permanently blocked from starting another
/// install. A drop guard runs no matter how the scope exits.
struct InstallRunningGuard;
impl Drop for InstallRunningGuard {
    fn drop(&mut self) {
        INSTALL_RUNNING.store(false, Ordering::SeqCst);
    }
}

// This is a Tauri `#[command]` IPC endpoint — its argument list is the wire
// protocol between the install wizard frontend and the backend. Packing
// the fields into a struct just forces every JS caller to build an object
// with the same keys, adding indirection without simplification.
//
// v0.3.0 removed the `api_key` parameter: the installer no longer collects
// the claw's API key. Each claw is responsible for its own credential UX
// post-install, from its own ClawPage management view. This keeps the
// installer a single-purpose tool (provisioning) and avoids baking a
// schema for one specific claw (openclaw) into every install path.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn install_openclaw(
    app: tauri::AppHandle,
    instance_name: String,
    claw_type: Option<String>,
    claw_version: String,
    use_native: bool,
    install_browser: bool,
    _install_mcp_bridge: Option<bool>,
    gateway_port: u16,
    image: Option<String>,
    proxy_json: Option<String>,
) -> Result<(), String> {
    if INSTALL_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Installation already in progress. Please wait for it to finish.".into());
    }

    let ct = claw_type.unwrap_or_else(|| "openclaw".into());
    // Keep the instance name around for the post-install instance-changed
    // emit — `instance_name` itself is moved into the CLI args vec below.
    let instance_name_for_emit = instance_name.clone();

    // Build v2 CLI args. Shape per CLI-DESIGN.md §7.1:
    //   clawcli install <claw> --name <N> --backend <native|lima|wsl2|podman>
    //                          --version <V> --port <P> [--browser]
    //
    // v1's `--mode sandbox|native` collapsed; v2 takes the backend
    // explicitly. `native` skips the VM; sandboxed defaults pick the
    // host's preferred backend (`lima`/`wsl2`/`podman`) when omitted.
    // `--image` (v1 offline-install) has no v2 equivalent yet —
    // dropped silently; the orchestrator stays online-only.
    let _ = image; // ack the wizard arg without using it
    let backend_arg = if use_native {
        "native"
    } else if cfg!(target_os = "macos") {
        "lima"
    } else if cfg!(target_os = "windows") {
        "wsl2"
    } else {
        "podman"
    };
    let mut args = vec![
        "install".to_string(),
        ct.clone(),
        "--name".to_string(), instance_name,
        "--backend".to_string(), backend_arg.to_string(),
        "--version".to_string(), claw_version,
        "--port".to_string(), gateway_port.to_string(),
    ];
    if install_browser {
        args.push("--browser".to_string());
    }

    // Translate the wizard's proxy selection into HTTP_PROXY / HTTPS_PROXY /
    // NO_PROXY env vars for the CLI subprocess. Deliberately not written to
    // config.toml — the user picked a proxy for this install only.
    // Resolver's `triple_from_config_proxy` handles auth + keychain lookup.
    let proxy_env: Vec<(String, String)> = proxy_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<ProxyConfig>(s).ok())
        .and_then(|p| triple_from_proxy_config(&p))
        .map(|t| vec![
            ("http_proxy".to_string(), t.http.clone()),
            ("HTTP_PROXY".to_string(), t.http),
            ("https_proxy".to_string(), t.https.clone()),
            ("HTTPS_PROXY".to_string(), t.https),
            ("no_proxy".to_string(), t.no_proxy.clone()),
            ("NO_PROXY".to_string(), t.no_proxy),
        ])
        .unwrap_or_default();

    let app_handle = app.clone();
    // Move the selection JSON into the task so the pre-flight probe sees the
    // exact same proxy the CLI subprocess will be launched under.
    let proxy_json_for_preflight = proxy_json.clone();
    tokio::spawn(async move {
        // Guard released when this task exits, via any path (normal return,
        // early return, panic caught by tokio's spawn, task cancel).
        let _guard = InstallRunningGuard;

        // Pre-flight connectivity gate. v0.3.0 contract: networking is the
        // user's problem, not ours. Before we spend several minutes
        // provisioning a VM / running apk + npm / pulling images, we do one
        // fast probe under the user's chosen proxy (or no proxy). If it
        // fails, we bail with a bilingual error rather than letting the
        // failure surface as an opaque "apk: BAD signature" 90 seconds in.
        //
        // Skipped when the wizard didn't collect a selection (proxy_json is
        // None). That only happens if the user got here through a path that
        // bypasses StepNetwork — we don't want to block those callers.
        if let Some(ref pj) = proxy_json_for_preflight {
            match crate::ipc::network::run_connectivity_probes(
                Some(&app_handle),
                Some(pj.as_str()),
            ).await {
                Ok(results) => {
                    let failed: Vec<String> = results.iter()
                        .filter(|r| !r.ok)
                        .map(|r| format!("{}: {}", r.endpoint, r.message))
                        .collect();
                    if !failed.is_empty() {
                        let err_msg = format!(
                            "网络不通，请先解决网络问题再重试安装 / Network unreachable — fix your network before retrying install.\n\n\
                            失败端点 / Failed endpoints:\n  - {}",
                            failed.join("\n  - "),
                        );
                        let _ = app_handle.emit("install-failed", &err_msg);
                        crate::tray::send_notification(
                            &app_handle,
                            "Install Blocked / 安装阻止",
                            "Network unreachable — see install window.",
                        );
                        return;
                    }
                }
                Err(e) => {
                    let err_msg = format!(
                        "连通性测试失败 / Connectivity test failed: {e}\n\n\
                        请检查代理设置与网络后重试。 / Check proxy settings and network, then retry.",
                    );
                    let _ = app_handle.emit("install-failed", &err_msg);
                    return;
                }
            }
        }

        let (tx, mut rx) = tokio::sync::mpsc::channel::<CliEvent>(32);

        // Forward CLI events to Tauri frontend
        let app_fwd = app_handle.clone();
        let fwd_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match &event {
                    CliEvent::Progress { .. } | CliEvent::Info { .. } => {
                        // Forward as structured event (Serialize derives available)
                        let _ = app_fwd.emit("install-progress", &event);
                    }
                    CliEvent::Complete { .. } => {
                        let _ = app_fwd.emit("install-progress", &event);
                    }
                    CliEvent::Error { .. } => {
                        let _ = app_fwd.emit("install-progress", &event);
                    }
                    CliEvent::Data { .. } => {
                        let _ = app_fwd.emit("install-progress", &event);
                    }
                }
            }
        });

        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let env_ref: Vec<(&str, String)> = proxy_env.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let result = cli_bridge::run_cli_streaming_with_env(&args_ref, &env_ref, tx, |_| {}).await;
        fwd_task.await.ok();

        match result {
            Ok(_) => {
                let _ = app_handle.emit("install-complete", ());
                // Canonical state-sync event. The install runs in an isolated
                // WebviewWindow, so the main window doesn't inspect config.toml
                // on its own — MainLayout's `instance-changed` listener is the
                // single code path that refreshes `instances()` and makes the
                // newly-installed entry appear in Home / ClawPage. The separate
                // front-end emit in App.tsx on window close is belt-and-braces;
                // this backend emit is the authoritative one that can't be
                // accidentally skipped if the install window is force-closed.
                emit_instance_changed(
                    &app_handle,
                    InstanceChanged::simple(InstanceAction::Install, &instance_name_for_emit),
                );
                crate::tray::send_notification(
                    &app_handle,
                    "Install Complete",
                    &format!("{ct} has been installed successfully"),
                );
            }
            Err(e) => {
                let err_msg = e.to_string();
                let _ = app_handle.emit("install-failed", &err_msg);
                crate::tray::send_notification(
                    &app_handle,
                    "Install Failed",
                    &format!("{ct} installation failed: {err_msg}"),
                );
            }
        }
        // _guard drops here and resets INSTALL_RUNNING — no manual store needed.
    });

    Ok(())
}

#[tauri::command]
pub async fn install_prerequisites(app: tauri::AppHandle) -> Result<(), String> {
    // v2: prerequisite installation is per-backend via SandboxOps. Run
    // through clawcli to keep one code path; `clawcli system install-prerequisites`
    // wires the same `ensure_prerequisites` flow with progress events.
    let _ = app.emit("prereq-step", "Installing sandbox prerequisites...");
    cli_bridge::run_cli(&["system", "install-prerequisites"]).await
        .map_err(|e| e.to_string())?;
    let _ = app.emit("prereq-step", "Prerequisites installed successfully");
    Ok(())
}

/// Translate a `ProxyConfig` (from the install wizard JSON) into a
/// `ProxyTriple` for env injection. Replaces v1's
/// `proxy_resolver::triple_from_config_proxy`. Loops up the password
/// from the keychain when `auth_required` is set.
fn triple_from_proxy_config(p: &ProxyConfig) -> Option<ProxyTriple> {
    if !p.enabled || p.http_proxy.is_empty() {
        return None;
    }
    // Inject password into the URL when auth is required and we have one
    // in the keychain. Best-effort — if the keychain lookup fails, the
    // bare URL is still returned and the user gets an upstream auth error.
    let inject_auth = |url: &str| -> String {
        if !p.auth_required || p.auth_user.is_empty() {
            return url.into();
        }
        let pw = credentials::get_proxy_password().unwrap_or_default();
        if pw.is_empty() {
            return url.into();
        }
        // Cheap parser: assume scheme://host... — splice user:pw@ after scheme://.
        if let Some(idx) = url.find("://") {
            let (scheme, rest) = url.split_at(idx + 3);
            return format!("{scheme}{}:{}@{rest}", p.auth_user, pw);
        }
        url.into()
    };
    let http = inject_auth(&p.http_proxy);
    let https = if p.https_proxy.is_empty() {
        http.clone()
    } else {
        inject_auth(&p.https_proxy)
    };
    let no_proxy = if p.no_proxy.is_empty() {
        "localhost,127.0.0.1".into()
    } else {
        p.no_proxy.clone()
    };
    Some(ProxyTriple { http, https, no_proxy, source: ProxySource::GlobalConfig })
}

#[derive(Serialize)]
pub struct SystemCheckInfo {
    pub os: String,
    pub arch: String,
    pub memory_gb: f64,
    pub disk_free_gb: f64,
    pub sandbox_backend: String,
    pub sandbox_available: bool,
    pub checks: Vec<ApiCheckItem>,
}

#[tauri::command]
pub async fn system_check() -> Result<SystemCheckInfo, String> {
    // v2: `clawcli system info` replaces v1's `system-check`. Same
    // payload (probes OS / memory / disk / sandbox availability),
    // re-typed as `SystemInfo` in v2 wire — the v1 alias here keeps
    // the GUI's existing IPC contract stable.
    let data = cli_bridge::run_cli(&["system", "info"]).await.map_err(|e| e.to_string())?;
    let resp: SystemCheckResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;

    Ok(SystemCheckInfo {
        os: resp.os,
        arch: resp.arch,
        memory_gb: resp.memory_gb,
        disk_free_gb: resp.disk_free_gb,
        sandbox_backend: resp.sandbox_backend.clone(),
        sandbox_available: resp.sandbox_available,
        checks: resp.checks,
    })
}

#[tauri::command]
pub async fn restart_computer() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        crate::util::silent_cmd("shutdown")
            .args(["/r", "/t", "5", "/c", "ClawEnv: Restarting to complete WSL2 installation"])
            .status()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err("Restart is only needed on Windows for WSL2 installation".into())
    }
}

/// Open file picker for import and return selected path
#[tauri::command]
pub async fn pick_import_file(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app.dialog().file()
        .add_filter("ClawEnv Package", &["tar.gz", "gz"])
        .blocking_pick_file();
    match path {
        Some(p) => Ok(p.to_string()),
        None => Err("No file selected".into()),
    }
}

/// Validate an import file name against current platform
#[tauri::command]
pub async fn validate_import_file(file_path: String) -> Result<serde_json::Value, String> {
    let filename = std::path::Path::new(&file_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    // Expected format: {platform}-{arch}-{timestamp}.tar.gz
    let parts: Vec<&str> = filename.split('-').collect();
    if parts.len() < 3 {
        return Ok(serde_json::json!({
            "valid": false,
            "error": "Unrecognized file name format. Expected: {platform}-{arch}-{timestamp}.tar.gz",
            "is_native": false,
        }));
    }

    let file_platform = parts[0];
    let file_arch = parts[1];

    // Determine if native or sandbox
    let is_native = matches!(file_platform, "windows" | "macos" | "linux");
    let is_sandbox = matches!(file_platform, "lima" | "wsl2" | "podman");

    if !is_native && !is_sandbox {
        return Ok(serde_json::json!({
            "valid": false,
            "error": format!("Unknown platform '{}' in file name", file_platform),
            "is_native": false,
        }));
    }

    // Check platform match
    let current_platform = if cfg!(target_os = "macos") { "macos" }
        else if cfg!(target_os = "windows") { "windows" }
        else { "linux" };
    let current_backend = if cfg!(target_os = "macos") { "lima" }
        else if cfg!(target_os = "windows") { "wsl2" }
        else { "podman" };

    let platform_ok = if is_native {
        file_platform == current_platform
    } else {
        file_platform == current_backend
    };

    // Check arch match
    let current_arch = std::env::consts::ARCH;
    let arch_ok = file_arch == current_arch
        || (file_arch == "arm64" && current_arch == "aarch64")
        || (file_arch == "aarch64" && current_arch == "aarch64")
        || (file_arch == "x64" && current_arch == "x86_64")
        || (file_arch == "x86_64" && current_arch == "x86_64");

    let mut errors = Vec::new();
    if !platform_ok {
        errors.push(format!("Platform mismatch: file is for '{}', this machine is '{}'",
            file_platform, if is_native { current_platform } else { current_backend }));
    }
    if !arch_ok {
        errors.push(format!("Architecture mismatch: file is for '{}', this machine is '{}'",
            file_arch, current_arch));
    }

    Ok(serde_json::json!({
        "valid": errors.is_empty(),
        "error": errors.join("; "),
        "is_native": is_native,
        "platform": file_platform,
        "arch": file_arch,
    }))
}

/// Check if a native instance already exists
#[tauri::command]
pub async fn has_native_instance() -> Result<bool, String> {
    use clawops_core::instance::{InstanceRegistry, SandboxKind};
    let registry = InstanceRegistry::with_default_path();
    let instances = registry.list().await.map_err(|e| e.to_string())?;
    Ok(instances.iter().any(|i| i.backend == SandboxKind::Native))
}
