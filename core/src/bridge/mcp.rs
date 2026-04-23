//! Local MCP server exposed by the bridge on `127.0.0.1`.
//!
//! Protocol: JSON-RPC 2.0 over HTTP POST `/mcp`. Supports MCP's
//! `initialize`, `tools/list`, `tools/call`. No SSE / no server-initiated
//! notifications in this revision — Claude Code / other MCP clients that
//! only do request/response work; clients that require streaming
//! notifications need `tools/list_changed` support, which is a follow-up.
//!
//! Discovery: `bridge.mcp.json` is written to `~/.clawenv/` with
//! `{url, token, pid}`. The claw instance (or a user copy-pasting) reads
//! it to configure its MCP client.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::input::ToolRegistry;
use crate::remote::audit::AuditLog;

#[derive(Clone)]
pub struct McpState {
    pub registry: ToolRegistry,
    pub token: String,
    pub audit: Option<Arc<AuditLog>>,
}

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    #[allow(dead_code)]
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

pub fn router(state: McpState) -> Router {
    Router::new()
        .route("/mcp", post(handle_rpc))
        .route("/mcp/health", get(handle_health))
        .with_state(state)
}

async fn handle_health() -> &'static str { "ok" }

async fn handle_rpc(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    let id = req.id.unwrap_or(Value::Null);

    if !state.token.is_empty() {
        let got = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let expected = format!("Bearer {}", state.token);
        if got != expected {
            return (
                StatusCode::UNAUTHORIZED,
                Json(err_resp(id, -32001, "unauthorized")),
            )
                .into_response();
        }
    }

    let payload = match req.method.as_str() {
        "initialize" => ok_resp(id, serde_json::json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {
                "name": "clawenv-input",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": { "tools": {} }
        })),
        "tools/list" => {
            let tools: Vec<_> = state.registry.list()
                .into_iter()
                .map(|s| serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "inputSchema": s.input_schema,
                }))
                .collect();
            ok_resp(id, serde_json::json!({ "tools": tools }))
        }
        "tools/call" => {
            #[derive(Deserialize)]
            struct Call {
                name: String,
                #[serde(default)]
                arguments: Value,
            }
            let call: Call = match serde_json::from_value(req.params) {
                Ok(c) => c,
                Err(e) => return Json(err_resp(id, -32602, &format!("invalid params: {e}"))).into_response(),
            };
            if let Some(a) = &state.audit {
                a.log(crate::remote::audit::AuditEvent::McpCall {
                    tool: call.name.clone(),
                });
            }
            match state.registry.call(&call.name, call.arguments).await {
                Ok(v) => {
                    let text = serde_json::to_string(&v).unwrap_or_else(|_| v.to_string());
                    ok_resp(id, serde_json::json!({
                        "content": [{"type":"text","text": text}],
                        "structuredContent": v,
                    }))
                }
                Err(e) => ok_resp(id, serde_json::json!({
                    "isError": true,
                    "content": [{
                        "type":"text",
                        "text": serde_json::json!({
                            "code": e.code(),
                            "message": e.to_string(),
                        }).to_string()
                    }]
                })),
            }
        }
        other => err_resp(id, -32601, &format!("method '{other}' not implemented")),
    };

    Json(payload).into_response()
}

fn ok_resp(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse { jsonrpc: "2.0", id, result: Some(result), error: None }
}

fn err_resp(id: Value, code: i32, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError { code, message: message.into() }),
    }
}

// ---------- Server lifecycle ----------

pub struct McpServerHandle {
    pub addr: SocketAddr,
    pub token: String,
    pub shutdown: tokio::sync::oneshot::Sender<()>,
    pub join: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl McpServerHandle {
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }
}

pub async fn start(
    registry: ToolRegistry,
    preferred_port: u16,
    audit: Option<Arc<AuditLog>>,
) -> anyhow::Result<McpServerHandle> {
    let token = random_token();
    let state = McpState { registry, token: token.clone(), audit };

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", preferred_port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(
                target: "clawenv::remote",
                "preferred port {preferred_port} unavailable ({e}); choosing random",
            );
            tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?
        }
    };
    let addr = listener.local_addr()?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app = router(state);

    let join = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { let _ = rx.await; })
            .await
    });

    Ok(McpServerHandle { addr, token, shutdown: tx, join })
}

fn random_token() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

// ---------- Discovery file ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeMcpDescriptor {
    pub url: String,
    pub token: String,
    pub pid: u32,
}

pub fn default_descriptor_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".clawenv")
        .join("bridge.mcp.json")
}

/// Errors from `write_descriptor`. Separate from io::Error so the
/// caller can distinguish "another live daemon owns this file" from
/// ordinary filesystem failures.
#[derive(Debug, thiserror::Error)]
pub enum DescriptorError {
    #[error("another clawenv bridge is already using {path} (pid {pid} is still alive). \
             Stop it before starting a new one, or delete the file if the pid is stale.")]
    LiveOwner { path: String, pid: u32 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Write `bridge.mcp.json` with mode 0600 on unix. On windows we rely on
/// per-user ACLs inherited from the home directory; we don't hand-roll
/// SetSecurityInfo here.
///
/// Before overwriting an existing descriptor, this checks whether the
/// recorded pid is still running. If it is, the write is refused — two
/// concurrent bridge daemons would silently overwrite each other's
/// tokens and both would stop working. Stale (crashed-daemon) files
/// are quietly replaced.
pub fn write_descriptor(path: &Path, desc: &BridgeMcpDescriptor) -> Result<(), DescriptorError> {
    // Liveness check on any existing descriptor.
    if path.exists() {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(existing) = serde_json::from_str::<BridgeMcpDescriptor>(&raw) {
                if existing.pid != std::process::id() && pid_is_alive(existing.pid) {
                    return Err(DescriptorError::LiveOwner {
                        path: path.display().to_string(),
                        pid: existing.pid,
                    });
                }
            }
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(desc)?;
    std::fs::write(path, bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Best-effort liveness check. On unix uses `kill(pid, 0)`. On windows
/// we currently skip the check — getting this right needs `OpenProcess`
/// + handle cleanup, which pulls in `windows-sys`; for now the windows
/// path always claims "alive" (conservative: refuses to overwrite, so
/// a user might see false-positive conflicts but never silent
/// corruption) EXCEPT when the recorded pid equals our own pid.
fn pid_is_alive(pid: u32) -> bool {
    // pid 0 is POSIX's "current process group", not a real pid —
    // kill(0, 0) would succeed and make us claim "alive". Defensive:
    // treat 0 as dead so the descriptor file written by a crashed
    // early-boot process (before std::process::id() could be written
    // correctly) is always recoverable.
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with sig=0 only tests the target without
        // signalling. EPERM means the process exists but we can't
        // signal it (different uid); still "alive" from our POV.
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if ret == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        !matches!(err.raw_os_error(), Some(libc::ESRCH))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, FALSE, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: OpenProcess returns NULL on failure (nonexistent pid
        // or access denied). We never dereference the handle, and we
        // always CloseHandle if we obtained one.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let ok = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
        let _ = unsafe { CloseHandle(handle) };
        if ok == 0 {
            // Couldn't read — be conservative and say "alive" so we
            // don't overwrite what might be a running peer.
            return true;
        }
        // Win32 convention: a still-running process reports STILL_ACTIVE.
        // Anything else is a real exit code. Note: in principle a
        // process could legitimately exit with code 259; the check is
        // heuristic but matches common practice.
        exit_code == STILL_ACTIVE as u32
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::ToolRegistry;

    #[tokio::test]
    async fn initialize_and_tools_list_roundtrip() {
        let reg = ToolRegistry::new(vec![]);
        let handle = start(reg, 0, None).await.unwrap();
        let url = handle.url();
        let token = handle.token.clone();

        let client = reqwest::Client::new();
        let init: Value = client.post(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "clawenv-input");

        let list: Value = client.post(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .send().await.unwrap().json().await.unwrap();
        assert!(list["result"]["tools"].is_array());

        let _ = handle.shutdown.send(());
        let _ = handle.join.await;
    }

    #[tokio::test]
    async fn unauthorised_without_token() {
        let reg = ToolRegistry::new(vec![]);
        let handle = start(reg, 0, None).await.unwrap();
        let url = handle.url();

        let resp = reqwest::Client::new().post(&url)
            .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
            .send().await.unwrap();
        assert_eq!(resp.status(), 401);

        let _ = handle.shutdown.send(());
        let _ = handle.join.await;
    }

    #[test]
    fn descriptor_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("bridge.mcp.json");
        let d = BridgeMcpDescriptor {
            url: "http://127.0.0.1:33721/mcp".into(),
            token: "abcd".into(),
            pid: std::process::id(), // self; liveness check must not fail on own pid
        };
        write_descriptor(&p, &d).unwrap();
        let raw: BridgeMcpDescriptor = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(raw.token, "abcd");
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_refuses_live_foreign_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("bridge.mcp.json");

        // Pretend another live pid owns the file: use pid=1 (init),
        // which is guaranteed to be alive on any running unix system
        // but isn't us.
        let hostage = BridgeMcpDescriptor {
            url: "http://stale/mcp".into(),
            token: "stale".into(),
            pid: 1,
        };
        std::fs::write(&p, serde_json::to_vec(&hostage).unwrap()).unwrap();

        let new_owner = BridgeMcpDescriptor {
            url: "http://fresh/mcp".into(),
            token: "fresh".into(),
            pid: std::process::id(),
        };
        let err = write_descriptor(&p, &new_owner).unwrap_err();
        match err {
            DescriptorError::LiveOwner { pid, .. } => assert_eq!(pid, 1),
            other => panic!("unexpected error: {other:?}"),
        }
        // File must be untouched.
        let raw: BridgeMcpDescriptor = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(raw.token, "stale");
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_overwrites_stale_pid() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("bridge.mcp.json");
        // pid=0 is always invalid/dead on unix.
        let hostage = BridgeMcpDescriptor {
            url: "http://stale/mcp".into(),
            token: "stale".into(),
            pid: 0,
        };
        std::fs::write(&p, serde_json::to_vec(&hostage).unwrap()).unwrap();

        let new_owner = BridgeMcpDescriptor {
            url: "http://fresh/mcp".into(),
            token: "fresh".into(),
            pid: std::process::id(),
        };
        write_descriptor(&p, &new_owner).expect("should overwrite stale pid file");
        let raw: BridgeMcpDescriptor = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(raw.token, "fresh");
    }
}
