use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, Notify, RwLock};

use super::permissions::{BridgePermissions, PermissionResult};

/// Opaque event-emitter callback signature used to forward bridge events
/// into whichever UI layer embeds the bridge (Tauri's frontend emit in
/// production, a test double in tests). Aliased because the bare type has
/// three trait bounds and reads poorly in struct fields and function args.
pub type EventEmitter = Box<dyn Fn(&str, &str) + Send + Sync>;

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
    pub event_emitter: Option<EventEmitter>,
    /// Registered hardware devices (in-memory, devices re-register on reconnect)
    pub hw_devices: Vec<HwDevice>,
    /// Broadcast channel for pushing notifications to ALL hardware WebSocket connections
    pub hw_notify_tx: broadcast::Sender<String>,
    /// Targeted channel: (device_id, payload) for pushing to a specific device
    pub hw_targeted_tx: broadcast::Sender<(String, String)>,
    /// Device IDs currently connected via WebSocket (for dedup vs HTTP callback)
    pub hw_ws_device_ids: HashSet<String>,
    /// Shared HTTP client for hardware callbacks (reuse connection pool)
    pub hw_http_client: reqwest::Client,
    /// Token for hardware API authentication (empty = no auth)
    pub hw_auth_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HilRequest {
    pub reason: String,
    #[serde(default)]
    pub url: String,
}

/// A registered hardware device that can receive notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HwDevice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub callback_url: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub registered_at: String,
    /// Last activity timestamp (register or heartbeat), for TTL cleanup.
    pub last_seen: String,
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
    /// Maximum number of entries to return (default: 1000, max: 10000).
    #[serde(default)]
    limit: Option<usize>,
    /// Number of entries to skip before returning results.
    #[serde(default)]
    offset: Option<usize>,
}

const FILE_LIST_DEFAULT_LIMIT: usize = 1000;
const FILE_LIST_MAX_LIMIT: usize = 10000;

#[derive(Serialize)]
struct DirEntry {
    name: String,
    is_dir: bool,
    size: u64,
}

#[derive(Serialize)]
struct FileListRes {
    entries: Vec<DirEntry>,
    /// Total number of entries in the directory (before pagination).
    total: usize,
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
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
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

    let limit = req.limit.unwrap_or(FILE_LIST_DEFAULT_LIMIT).min(FILE_LIST_MAX_LIMIT);
    let offset = req.offset.unwrap_or(0);

    let mut all_entries = Vec::new();
    let mut dir = tokio::fs::read_dir(&path)
        .await
        .map_err(|e| err_json(format!("Failed to list directory: {e}")))?;

    while let Ok(Some(entry)) = dir.next_entry().await {
        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue, // Skip entries with unreadable metadata (broken symlinks)
        };
        all_entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }

    let total = all_entries.len();
    let entries: Vec<DirEntry> = all_entries.into_iter().skip(offset).take(limit).collect();

    Ok(Json(FileListRes { entries, total }))
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

// ---------- Hardware device handlers ----------

/// Device registration TTL: devices not seen for 30 minutes are cleaned up.
const HW_DEVICE_TTL_SECS: i64 = 1800;

/// Generate a collision-resistant device ID: timestamp_ms + 4 random hex digits.
fn generate_device_id() -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let rand_part: u16 = (ts as u16).wrapping_mul(0x9E37).wrapping_add(
        std::process::id() as u16
    ) ^ (ts >> 16) as u16;
    format!("hw-{ts:013x}-{rand_part:04x}")
}

/// Check X-ClawEnv-HW-Token header. Returns Err if token is configured but missing/wrong.
fn check_hw_auth(state: &BridgeState, headers: &axum::http::HeaderMap) -> Result<(), (StatusCode, Json<ErrorRes>)> {
    if state.hw_auth_token.is_empty() {
        return Ok(()); // no auth configured
    }
    let provided = headers
        .get("X-ClawEnv-HW-Token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided == state.hw_auth_token {
        Ok(())
    } else {
        Err((StatusCode::UNAUTHORIZED, Json(ErrorRes { error: "Invalid or missing X-ClawEnv-HW-Token".into() })))
    }
}

/// Remove devices not seen for more than HW_DEVICE_TTL_SECS.
fn cleanup_stale_devices(devices: &mut Vec<HwDevice>) {
    let now = chrono::Utc::now();
    let before = devices.len();
    devices.retain(|d| {
        chrono::DateTime::parse_from_rfc3339(&d.last_seen)
            .map(|t| (now - t.with_timezone(&chrono::Utc)).num_seconds() < HW_DEVICE_TTL_SECS)
            .unwrap_or(false) // malformed timestamp → remove
    });
    let removed = before - devices.len();
    if removed > 0 {
        tracing::info!("Cleaned up {removed} stale hardware device(s)");
    }
}

#[derive(Deserialize)]
struct HwRegisterReq {
    name: String,
    #[serde(default)]
    callback_url: String,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Serialize)]
struct HwRegisterRes {
    ok: bool,
    device_id: String,
}

#[derive(Deserialize)]
struct HwUnregisterReq {
    device_id: String,
}

#[derive(Deserialize)]
struct HwNotifyReq {
    message: String,
    #[serde(default = "default_notify_level")]
    level: String,
    #[serde(default)]
    device_id: String,
    #[serde(default)]
    from_instance: String,
}

fn default_notify_level() -> String { "info".into() }

#[derive(Serialize)]
struct HwNotifyRes {
    ok: bool,
    ws_delivered: usize,
    http_callbacks_sent: usize,
}

#[derive(Serialize)]
struct HwDeviceListRes {
    devices: Vec<HwDevice>,
}

/// POST /api/hw/register — hardware device registers itself
async fn hw_register_handler(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<HwRegisterReq>,
) -> Result<Json<HwRegisterRes>, (StatusCode, Json<ErrorRes>)> {
    {
        let s = state.read().await;
        check_hw_auth(&s, &headers)?;
    }
    let now = chrono::Utc::now().to_rfc3339();
    let device_id = generate_device_id();
    let device = HwDevice {
        id: device_id.clone(),
        name: req.name,
        callback_url: req.callback_url,
        capabilities: req.capabilities,
        registered_at: now.clone(),
        last_seen: now,
    };
    let mut s = state.write().await;
    cleanup_stale_devices(&mut s.hw_devices);
    tracing::info!("Hardware device registered: {} ({})", device.name, device.id);
    s.hw_devices.push(device);
    Ok(Json(HwRegisterRes { ok: true, device_id }))
}

/// POST /api/hw/unregister — hardware device unregisters
async fn hw_unregister_handler(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<HwUnregisterReq>,
) -> Result<Json<OkRes>, (StatusCode, Json<ErrorRes>)> {
    {
        let s = state.read().await;
        check_hw_auth(&s, &headers)?;
    }
    let mut s = state.write().await;
    let before = s.hw_devices.len();
    s.hw_devices.retain(|d| d.id != req.device_id);
    s.hw_ws_device_ids.remove(&req.device_id);
    let removed = s.hw_devices.len() < before;
    if removed {
        tracing::info!("Hardware device unregistered: {}", req.device_id);
    }
    Ok(Json(OkRes { ok: removed }))
}

/// GET /api/hw/devices — list registered hardware devices
async fn hw_devices_handler(
    State(state): State<SharedState>,
) -> Json<HwDeviceListRes> {
    let s = state.read().await;
    Json(HwDeviceListRes { devices: s.hw_devices.clone() })
}

/// POST /api/hw/notify — MCP plugin calls this to push notification to devices.
/// Broadcasts via WebSocket to connected devices; HTTP callback only for non-WS devices.
async fn hw_notify_handler(
    State(state): State<SharedState>,
    Json(req): Json<HwNotifyReq>,
) -> Result<Json<HwNotifyRes>, (StatusCode, Json<ErrorRes>)> {
    let payload = serde_json::json!({
        "type": "notify",
        "message": req.message,
        "level": req.level,
        "device_id": req.device_id,
        "from_instance": req.from_instance,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }).to_string();

    let s = state.read().await;

    // WS delivery: broadcast to all, or targeted to specific device
    let target_all = req.device_id.is_empty() || req.device_id == "*";
    let ws_delivered = if target_all {
        let count = s.hw_notify_tx.receiver_count();
        if count > 0 {
            let _ = s.hw_notify_tx.send(payload.clone());
        }
        count
    } else if s.hw_ws_device_ids.contains(&req.device_id) {
        let _ = s.hw_targeted_tx.send((req.device_id.clone(), payload.clone()));
        1
    } else {
        0
    };

    // HTTP callback fallback — only for devices NOT connected via WS (B2 fix)
    let callback_devices: Vec<HwDevice> = s.hw_devices.iter()
        .filter(|d| {
            let id_match = target_all || d.id == req.device_id;
            let has_url = !d.callback_url.is_empty();
            let not_on_ws = !s.hw_ws_device_ids.contains(&d.id);
            id_match && has_url && not_on_ws
        })
        .cloned()
        .collect();
    let http_client = s.hw_http_client.clone(); // B3 fix: reuse client
    drop(s);

    let http_callbacks_sent = callback_devices.len();
    for device in callback_devices {
        let url = device.callback_url;
        let body = payload.clone();
        let client = http_client.clone();
        tokio::spawn(async move {
            if let Err(e) = client.post(&url)
                .header("Content-Type", "application/json")
                .body(body)
                .timeout(std::time::Duration::from_secs(5))
                .send().await
            {
                tracing::warn!("HW callback to {} failed: {e}", url);
            }
        });
    }

    Ok(Json(HwNotifyRes { ok: true, ws_delivered, http_callbacks_sent }))
}

/// GET /ws/hw?device_id=xxx — WebSocket upgrade for hardware device long connections.
/// Query param `device_id` associates this connection with a registered device (I4).
async fn hw_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorRes>)> {
    // Auth check via query param (WS can't easily set custom headers)
    {
        let s = state.read().await;
        if !s.hw_auth_token.is_empty() {
            let token = params.get("token").map(|s| s.as_str()).unwrap_or("");
            if token != s.hw_auth_token {
                return Err((StatusCode::UNAUTHORIZED, Json(ErrorRes {
                    error: "Invalid or missing token query parameter".into(),
                })));
            }
        }
    }
    let device_id = params.get("device_id").cloned().unwrap_or_default();
    Ok(ws.on_upgrade(move |socket| hw_ws_connection(socket, state, device_id)))
}

async fn hw_ws_connection(mut socket: WebSocket, state: SharedState, device_id: String) {
    let (mut rx, mut targeted_rx) = {
        let mut s = state.write().await;
        if !device_id.is_empty() {
            s.hw_ws_device_ids.insert(device_id.clone());
            // Touch last_seen on WS connect
            if let Some(dev) = s.hw_devices.iter_mut().find(|d| d.id == device_id) {
                dev.last_seen = chrono::Utc::now().to_rfc3339();
            }
        }
        (s.hw_notify_tx.subscribe(), s.hw_targeted_tx.subscribe())
    };
    tracing::info!("Hardware WS client connected (device_id={device_id:?})");

    loop {
        tokio::select! {
            // Forward broadcast notifications
            msg = rx.recv() => {
                match msg {
                    Ok(text) => {
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("HW WS client lagged, skipped {n} messages");
                    }
                    Err(_) => break,
                }
            }
            // Forward targeted notifications (matched by device_id)
            msg = targeted_rx.recv() => {
                match msg {
                    Ok((target_id, text)) => {
                        if target_id == device_id
                            && socket.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            // Handle incoming messages from hw client.
            // Clippy 1.95+ suggests collapsing the inner `if` into a
            // match guard — but `data` is `axum::body::Bytes` (no Copy)
            // so it can't be moved inside a guard (E0507). Keep the
            // nested form with an explicit allow + reason.
            msg = socket.recv() => {
                match msg {
                    #[allow(clippy::collapsible_match, clippy::collapsible_if,
                            reason = "pattern guard can't move `data`; see E0507")]
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    // Cleanup on disconnect
    if !device_id.is_empty() {
        let mut s = state.write().await;
        s.hw_ws_device_ids.remove(&device_id);
    }
    tracing::info!("Hardware WS client disconnected (device_id={device_id:?})");
}

// ---------- Server entry point ----------

pub async fn start_bridge(
    port: u16,
    permissions: BridgePermissions,
    event_emitter: Option<EventEmitter>,
    hw_auth_token: String,
    mcp: Option<super::mcp::McpState>,
) -> anyhow::Result<()> {
    let (hw_notify_tx, _) = broadcast::channel::<String>(64);
    let (hw_targeted_tx, _) = broadcast::channel::<(String, String)>(64);

    let state: SharedState = Arc::new(RwLock::new(BridgeState {
        permissions,
        hil_complete: Arc::new(Notify::new()),
        hil_pending: None,
        approval_notify: Arc::new(Notify::new()),
        approval_decision: None,
        approval_pending: None,
        event_emitter,
        hw_devices: Vec::new(),
        hw_notify_tx,
        hw_targeted_tx,
        hw_ws_device_ids: HashSet::new(),
        hw_http_client: reqwest::Client::new(),
        hw_auth_token,
    }));

    let mut app = Router::new()
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
        // Hardware device endpoints
        .route("/api/hw/register", post(hw_register_handler))
        .route("/api/hw/unregister", post(hw_unregister_handler))
        .route("/api/hw/devices", get(hw_devices_handler))
        .route("/api/hw/notify", post(hw_notify_handler))
        .route("/ws/hw", get(hw_ws_handler))
        .with_state(state);

    // Mount the MCP sub-router on the same listener when an input
    // ToolRegistry was supplied. Each sub-router carries its own state
    // (Bridge's RwLock'd SharedState here, MCP's per-launch token
    // there) so the two coexist without sharing typed state.
    if let Some(mcp_state) = mcp {
        app = app.merge(super::mcp::router(mcp_state));
    }

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("Bridge server listening on 0.0.0.0:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
