// Legacy SSH terminal - kept for developer CLI access.
// The UI now uses ttyd WebSocket (direct xterm.js connection).

use std::collections::HashMap;
use std::sync::Mutex;

use clawenv_core::config::ConfigManager;
use clawenv_core::manager::instance;
use once_cell::sync::Lazy;
use tauri::Emitter;

// Global terminal sessions: child processes (for kill) and stdin handles (for write)
pub static TERMINAL_CHILDREN: Lazy<Mutex<HashMap<String, tokio::process::Child>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub static TERMINAL_STDINS: Lazy<tokio::sync::Mutex<HashMap<String, tokio::process::ChildStdin>>> =
    Lazy::new(|| tokio::sync::Mutex::new(HashMap::new()));

#[tauri::command]
pub async fn start_terminal(
    app: tauri::AppHandle,
    instance_name: String,
) -> Result<String, String> {
    let config = ConfigManager::load().map_err(|e| e.to_string())?;
    let inst = instance::get_instance(&config, &instance_name).map_err(|e| e.to_string())?;

    // Determine shell command based on sandbox type
    // Use SSH -tt for Lima (forces PTY allocation = proper terminal with echo/prompt)
    // For Podman, use podman exec -it which also allocates PTY
    let (program, args) = match inst.sandbox_type {
        clawenv_core::sandbox::SandboxType::LimaAlpine => {
            // SSH directly to Lima VM as clawenv user with forced PTY
            let vm_name = format!("clawenv-{}", instance_name);
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            let key_path = format!("{}/.lima/_config/user", home);

            // Get SSH port from Lima
            let port_output = tokio::process::Command::new("limactl")
                .args(["list", "--format", "{{.SSHLocalPort}}", &vm_name])
                .output().await.map_err(|e| e.to_string())?;
            let port = String::from_utf8_lossy(&port_output.stdout).trim().to_string();
            let port = if port.is_empty() { "22".to_string() } else { port };

            ("ssh", vec![
                "-tt".to_string(),
                "-o".to_string(), "StrictHostKeyChecking=no".to_string(),
                "-o".to_string(), "UserKnownHostsFile=/dev/null".to_string(),
                "-o".to_string(), "LogLevel=ERROR".to_string(),
                "-i".to_string(), key_path,
                "-p".to_string(), port,
                "clawenv@127.0.0.1".to_string(),
            ])
        }
        clawenv_core::sandbox::SandboxType::PodmanAlpine => (
            "podman",
            vec![
                "exec".to_string(),
                "-it".to_string(),
                format!("clawenv-{}", instance_name),
                "sh".to_string(),
            ],
        ),
        clawenv_core::sandbox::SandboxType::Wsl2Alpine => (
            "wsl",
            vec![
                "-d".to_string(),
                format!("ClawEnv-Alpine-{}", instance_name),
            ],
        ),
        clawenv_core::sandbox::SandboxType::Native => ("sh", vec!["-i".to_string()]),
    };

    let session_id = format!(
        "term-{}-{}",
        instance_name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    tracing::info!("Starting terminal: {} {:?}", program, args);

    // SSH -tt: echo goes to stderr, command output to stdout.
    // Must read both and merge into one stream for xterm.js.
    let mut child = tokio::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            tracing::error!("Failed to spawn terminal: {e}");
            e.to_string()
        })?;

    tracing::info!("Terminal spawned: {} {:?}, session_id: {}", program, args, session_id);

    // Stream both stdout and stderr to frontend (merged)
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Read stdout
    let sid1 = session_id.clone();
    let app2 = app.clone();
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        if let Some(mut r) = stdout {
            let mut buf = [0u8; 4096];
            loop {
                match r.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = app2.emit("terminal-output", serde_json::json!({
                            "session_id": sid1, "data": String::from_utf8_lossy(&buf[..n]),
                        }));
                    }
                    Err(_) => break,
                }
            }
        }
    });

    // Read stderr (where PTY echo + prompts go)
    let sid2 = session_id.clone();
    let app3 = app.clone();
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        if let Some(mut r) = stderr {
            let mut buf = [0u8; 4096];
            loop {
                match r.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let _ = app3.emit("terminal-output", serde_json::json!({
                            "session_id": sid2, "data": String::from_utf8_lossy(&buf[..n]),
                        }));
                    }
                    Err(_) => break,
                }
            }
        }
    });

    // Store stdin separately (behind tokio::sync::Mutex for async access)
    let stdin = child.stdin.take();
    TERMINAL_CHILDREN
        .lock()
        .unwrap()
        .insert(session_id.clone(), child);
    if let Some(stdin_handle) = stdin {
        TERMINAL_STDINS
            .lock()
            .await
            .insert(session_id.clone(), stdin_handle);
    }

    Ok(session_id)
}

#[tauri::command]
pub async fn write_terminal(session_id: String, data: String) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut stdins = TERMINAL_STDINS.lock().await;
    if let Some(stdin) = stdins.get_mut(&session_id) {
        stdin
            .write_all(data.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stdin.flush().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn close_terminal(session_id: String) -> Result<(), String> {
    // Remove stdin handle first
    TERMINAL_STDINS.lock().await.remove(&session_id);
    // Remove and kill child process (drop lock before await)
    let child = TERMINAL_CHILDREN.lock()
        .map_err(|e| format!("Terminal state corrupted: {e}"))?
        .remove(&session_id);
    if let Some(mut child) = child {
        let _ = child.kill().await;
    }
    Ok(())
}
