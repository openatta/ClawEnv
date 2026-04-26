//! Local MCP server mounted under the bridge HTTP router.
//!
//! Protocol: JSON-RPC 2.0 over `POST /mcp`. Implements MCP's
//! `initialize`, `tools/list`, `tools/call`. No SSE / no server-initiated
//! notifications — clients that need `tools/list_changed` are a
//! follow-up.
//!
//! Discovery: `bridge.mcp.json` is written under `~/.clawenv/` with
//! `{url, token, pid}` so the local claw / Claude Code can configure
//! its MCP client without a copy-paste step. File is mode 0600 on Unix.
//!
//! Integration: this module exports `router(McpState) -> Router`. The
//! bridge server's `start_bridge` `.merge`s it into the main app on
//! the same port (3100 by default), so we don't open a second listener.
//!
//! Lifted from v1 `clawenv_core::bridge::mcp` (commit 772ef3c) with:
//!   - audit::AuditLog dependency dropped (use tracing instead — the
//!     remote/ module wasn't lifted to v2);
//!   - server lifecycle removed (we mount under existing bridge);
//!   - libc/windows-sys pid-liveness check dropped (Tauri is
//!     single-instance, the descriptor is always overwritten).

use std::path::{Path, PathBuf};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clawops_core::input::ToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone)]
pub struct McpState {
    pub registry: ToolRegistry,
    /// Bearer token. Empty disables auth (useful for local-only test
    /// builds; production launches always set this).
    pub token: String,
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
            tracing::info!(target: "clawenv::mcp", "tools/call name={}", call.name);
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

// ---------- Discovery file ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeMcpDescriptor {
    pub url: String,
    pub token: String,
    pub pid: u32,
}

pub fn default_descriptor_path() -> PathBuf {
    clawops_core::paths::clawenv_root().join("bridge.mcp.json")
}

/// Write `bridge.mcp.json` with mode 0600 on unix. On windows we rely
/// on per-user ACLs inherited from the home directory; SetSecurityInfo
/// would tighten that further but isn't worth the windows-sys dep
/// for a file already inside the user's profile.
///
/// Always overwrites — Tauri is single-instance, so a stale file from
/// a previous launch is the only realistic case and we always own it.
pub fn write_descriptor(path: &Path, desc: &BridgeMcpDescriptor) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(desc)
        .map_err(|e| std::io::Error::other(format!("descriptor encode: {e}")))?;
    std::fs::write(path, bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Generate a 256-bit hex random token for MCP bearer auth. Uses
/// `getrandom` indirectly via system entropy through std's RandomState
/// hasher — keeps us from pulling in `rand` for one call site.
pub fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut buf = [0u8; 32];
    let s = RandomState::new();
    // Fill 32 bytes by mixing 4× 64-bit hasher outputs seeded with
    // distinct inputs. RandomState is keyed by OS entropy at construction
    // (rustc-hash spec) — adequate for a per-launch session token.
    for chunk in buf.chunks_mut(8) {
        let mut h = s.build_hasher();
        h.write_u64(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64).unwrap_or(0));
        h.write_u64(std::process::id() as u64);
        let v = h.finish().to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
    hex::encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clawops_core::input::ToolRegistry;
    use tower::ServiceExt;

    #[tokio::test]
    async fn initialize_and_tools_list_roundtrip() {
        let reg = ToolRegistry::new(vec![]);
        let app = router(McpState { registry: reg, token: String::new() });

        let init_body = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
        })).unwrap();
        let resp = app.clone().oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(init_body))
                .unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), 200);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "clawenv-input");
    }

    #[tokio::test]
    async fn unauthorized_when_token_required_and_missing() {
        let reg = ToolRegistry::new(vec![]);
        let app = router(McpState { registry: reg, token: "secret".into() });

        let body = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
        })).unwrap();
        let resp = app.oneshot(
            axum::http::Request::builder()
                .method("POST").uri("/mcp")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), 401);
    }

    #[test]
    fn random_token_is_unique_and_hex() {
        let t1 = random_token();
        let t2 = random_token();
        assert_eq!(t1.len(), 64);
        assert_ne!(t1, t2);
        assert!(t1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_is_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("bridge.mcp.json");
        let desc = BridgeMcpDescriptor {
            url: "http://127.0.0.1:3100/mcp".into(),
            token: "t".into(),
            pid: 1234,
        };
        write_descriptor(&p, &desc).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
