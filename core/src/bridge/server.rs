use axum::{extract::State, http::StatusCode, routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Notify, RwLock};

use super::permissions::{BridgePermissions, PermissionResult};

pub struct BridgeState {
    pub permissions: BridgePermissions,
    /// HIL: signals when human intervention is complete
    pub hil_complete: Arc<Notify>,
    /// HIL: current pending request (reason, url)
    pub hil_pending: Option<HilRequest>,
    /// Exec approval: signals when user approves/denies
    pub approval_notify: Arc<Notify>,
    /// Exec approval: user decision (true=approved, false=denied)
    pub approval_decision: Option<bool>,
    /// Exec approval: pending command for display
    pub approval_pending: Option<String>,
    /// Tauri app handle for emitting events to frontend
    pub event_emitter: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HilRequest {
    pub reason: String,
    #[serde(default)]
    pub url: String,
}

type SharedState = Arc<RwLock<BridgeState>>;

// ---------- Request / Response types ----------

#[derive(Deserialize)]
struct FileReadReq {
    path: String,
}

#[derive(Serialize)]
struct FileReadRes {
    content: String,
}

#[derive(Deserialize)]
struct FileWriteReq {
    path: String,
    content: String,
}

#[derive(Serialize)]
struct OkRes {
    ok: bool,
}

#[derive(Deserialize)]
struct FileListReq {
    path: String,
}

#[derive(Serialize)]
struct DirEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize)]
struct FileListRes {
    entries: Vec<DirEntry>,
}

#[derive(Deserialize)]
struct ExecReq {
    command: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Serialize)]
struct ExecRes {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

#[derive(Serialize)]
struct HealthRes {
    status: String,
    version: String,
}

#[derive(Serialize)]
struct ErrorRes {
    error: String,
}

// ---------- Helpers ----------

fn expand_home(p: &str) -> PathBuf {
    if p.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&p[2..]);
        }
    }
    PathBuf::from(p)
}

fn err_json(msg: impl Into<String>) -> (StatusCode, Json<ErrorRes>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorRes {
            error: msg.into(),
        }),
    )
}

fn forbidden_json(msg: impl Into<String>) -> (StatusCode, Json<ErrorRes>) {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorRes {
            error: msg.into(),
        }),
    )
}

// ---------- Handlers ----------

async fn health_handler() -> Json<HealthRes> {
    Json(HealthRes {
        status: "ok".into(),
        version: "0.1.0".into(),
    })
}

async fn permissions_handler(State(state): State<SharedState>) -> Json<BridgePermissions> {
    let s = state.read().await;
    Json(s.permissions.clone())
}

async fn file_read_handler(
    State(state): State<SharedState>,
    Json(req): Json<FileReadReq>,
) -> Result<Json<FileReadRes>, (StatusCode, Json<ErrorRes>)> {
    let path = expand_home(&req.path);
    let perms = &state.read().await.permissions;

    match perms.can_read_file(&path) {
        PermissionResult::Allowed => {}
        PermissionResult::Denied(reason) => return Err(forbidden_json(reason)),
        PermissionResult::RequiresApproval(reason) => return Err(forbidden_json(reason)),
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| err_json(format!("Failed to read file: {e}")))?;

    Ok(Json(FileReadRes { content }))
}

async fn file_write_handler(
    State(state): State<SharedState>,
    Json(req): Json<FileWriteReq>,
) -> Result<Json<OkRes>, (StatusCode, Json<ErrorRes>)> {
    let path = expand_home(&req.path);
    let perms = &state.read().await.permissions;

    match perms.can_write_file(&path) {
        PermissionResult::Allowed => {}
        PermissionResult::Denied(reason) => return Err(forbidden_json(reason)),
        PermissionResult::RequiresApproval(reason) => return Err(forbidden_json(reason)),
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| err_json(format!("Failed to create directory: {e}")))?;
    }

    tokio::fs::write(&path, &req.content)
        .await
        .map_err(|e| err_json(format!("Failed to write file: {e}")))?;

    Ok(Json(OkRes { ok: true }))
}

async fn file_list_handler(
    State(state): State<SharedState>,
    Json(req): Json<FileListReq>,
) -> Result<Json<FileListRes>, (StatusCode, Json<ErrorRes>)> {
    let path = expand_home(&req.path);
    let perms = &state.read().await.permissions;

    match perms.can_read_file(&path) {
        PermissionResult::Allowed => {}
        PermissionResult::Denied(reason) => return Err(forbidden_json(reason)),
        PermissionResult::RequiresApproval(reason) => return Err(forbidden_json(reason)),
    }

    let mut entries = Vec::new();
    let mut dir = tokio::fs::read_dir(&path)
        .await
        .map_err(|e| err_json(format!("Failed to list directory: {e}")))?;

    while let Ok(Some(entry)) = dir.next_entry().await {
        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue, // Skip entries with unreadable metadata (broken symlinks)
        };
        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }

    Ok(Json(FileListRes { entries }))
}

async fn exec_handler(
    State(state): State<SharedState>,
    Json(req): Json<ExecReq>,
) -> Result<Json<ExecRes>, (StatusCode, Json<ErrorRes>)> {
    let perms = &state.read().await.permissions;

    // Build the full command string for permission checking
    let full_cmd = if req.args.is_empty() {
        req.command.clone()
    } else {
        format!("{} {}", req.command, req.args.join(" "))
    };

    match perms.can_exec(&full_cmd) {
        PermissionResult::Allowed => {}
        PermissionResult::Denied(reason) => return Err(forbidden_json(reason)),
        PermissionResult::RequiresApproval(_) => {
            // Store pending command and emit approval request to frontend
            let notify = {
                let mut s = state.write().await;
                s.approval_pending = Some(full_cmd.clone());
                s.approval_decision = None;
                if let Some(ref emitter) = s.event_emitter {
                    let payload = serde_json::json!({ "command": full_cmd }).to_string();
                    emitter("exec-approval-required", &payload);
                }
                s.approval_notify.clone()
            };
            // Block until user approves/denies (60s timeout)
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                notify.notified(),
            ).await;
            let approved = {
                let mut s = state.write().await;
                let decision = s.approval_decision.unwrap_or(false);
                s.approval_pending = None;
                s.approval_decision = None;
                decision
            };
            match result {
                Ok(()) if approved => {} // proceed to execute
                Ok(()) => return Err(forbidden_json("User denied the command".to_string())),
                Err(_) => return Err(forbidden_json("Approval timed out (60s)".to_string())),
            }
        }
    }

    let output = tokio::process::Command::new(&req.command)
        .args(&req.args)
        .output()
        .await
        .map_err(|e| err_json(format!("Failed to execute command: {e}")))?;

    Ok(Json(ExecRes {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code().unwrap_or(-1),
    }))
}

// ---------- HIL (Human-in-the-Loop) handlers ----------

#[derive(Serialize)]
struct HilStatusRes {
    active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Serialize)]
struct HilCompleteRes {
    ok: bool,
    message: String,
}

/// POST /api/hil/request — called by hil-skill.mjs from sandbox.
/// Blocks until user completes intervention (clicks "Continue Auto").
async fn hil_request_handler(
    State(state): State<SharedState>,
    Json(req): Json<HilRequest>,
) -> Result<Json<HilCompleteRes>, (StatusCode, Json<serde_json::Value>)> {
    let notify = {
        let mut s = state.write().await;
        s.hil_pending = Some(req.clone());
        // Emit event to Tauri frontend
        if let Some(ref emitter) = s.event_emitter {
            let payload = serde_json::json!({
                "reason": req.reason,
                "url": req.url,
            }).to_string();
            emitter("hil-bridge-request", &payload);
        }
        tracing::info!("HIL requested: {} (url: {})", req.reason, req.url);
        s.hil_complete.clone()
    };

    // Block until notified (user clicks "Continue Auto") or 10 min timeout
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(600),
        notify.notified(),
    ).await;

    // Clear pending state
    {
        let mut s = state.write().await;
        s.hil_pending = None;
    }

    match result {
        Ok(()) => Ok(Json(HilCompleteRes {
            ok: true,
            message: "Human intervention completed successfully.".into(),
        })),
        Err(_) => Ok(Json(HilCompleteRes {
            ok: false,
            message: "HIL request timed out after 10 minutes.".into(),
        })),
    }
}

/// GET /api/hil/status — check if HIL is active
async fn hil_status_handler(
    State(state): State<SharedState>,
) -> Json<HilStatusRes> {
    let s = state.read().await;
    match &s.hil_pending {
        Some(req) => Json(HilStatusRes {
            active: true,
            reason: Some(req.reason.clone()),
            url: Some(req.url.clone()),
        }),
        None => Json(HilStatusRes {
            active: false,
            reason: None,
            url: None,
        }),
    }
}

/// POST /api/hil/complete — called by ClawEnv GUI when user finishes
async fn hil_complete_handler(
    State(state): State<SharedState>,
) -> Json<HilCompleteRes> {
    let s = state.read().await;
    s.hil_complete.notify_one();
    tracing::info!("HIL completed by user");
    Json(HilCompleteRes { ok: true, message: "Notified agent to continue.".into() })
}

// ---------- Exec approval handlers ----------

#[derive(Serialize)]
struct ApprovalStatusRes {
    pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

/// POST /api/exec/approve — user approves the pending command
async fn exec_approve_handler(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let mut s = state.write().await;
    s.approval_decision = Some(true);
    s.approval_notify.notify_one();
    tracing::info!("Exec approved by user");
    Json(serde_json::json!({ "ok": true }))
}

/// POST /api/exec/deny — user denies the pending command
async fn exec_deny_handler(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let mut s = state.write().await;
    s.approval_decision = Some(false);
    s.approval_notify.notify_one();
    tracing::info!("Exec denied by user");
    Json(serde_json::json!({ "ok": true }))
}

/// GET /api/exec/pending — check if there's a pending approval
async fn exec_pending_handler(State(state): State<SharedState>) -> Json<ApprovalStatusRes> {
    let s = state.read().await;
    Json(ApprovalStatusRes {
        pending: s.approval_pending.is_some(),
        command: s.approval_pending.clone(),
    })
}

// ---------- Server entry point ----------

pub async fn start_bridge(
    port: u16,
    permissions: BridgePermissions,
    event_emitter: Option<Box<dyn Fn(&str, &str) + Send + Sync>>,
) -> anyhow::Result<()> {
    let state: SharedState = Arc::new(RwLock::new(BridgeState {
        permissions,
        hil_complete: Arc::new(Notify::new()),
        hil_pending: None,
        approval_notify: Arc::new(Notify::new()),
        approval_decision: None,
        approval_pending: None,
        event_emitter,
    }));

    let app = Router::new()
        .route("/api/health", get(health_handler))
        .route("/api/permissions", get(permissions_handler))
        .route("/api/file/read", post(file_read_handler))
        .route("/api/file/write", post(file_write_handler))
        .route("/api/file/list", post(file_list_handler))
        .route("/api/exec", post(exec_handler))
        .route("/api/exec/approve", post(exec_approve_handler))
        .route("/api/exec/deny", post(exec_deny_handler))
        .route("/api/exec/pending", get(exec_pending_handler))
        .route("/api/hil/request", post(hil_request_handler))
        .route("/api/hil/status", get(hil_status_handler))
        .route("/api/hil/complete", post(hil_complete_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("Bridge server listening on 0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
