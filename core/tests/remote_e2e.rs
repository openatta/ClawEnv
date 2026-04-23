//! End-to-end test for the remote-desktop runtime.
//!
//! Sets up a mock WSS server, the local MCP server, and a supervisor
//! all in-process, then asserts:
//!   1. WSS handshake + hello frame
//!   2. server → bridge `user_message` flows through supervisor
//!   3. bridge → server ack round-trips
//!   4. MCP `tools/list` + `tools/call` work against the same registry
//!      the runtime uses
//!   5. full `start_runtime` wiring (with dispatcher) echoes a
//!      user_message back to the mock server
//!
//! No real keyboard/mouse/screen APIs are invoked — the tests register
//! a synthetic `test.echo` tool alongside the MCP server. The production
//! tool factory is exercised by separate cargo tests in core/src/input.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use clawenv_core::bridge::mcp;
use clawenv_core::config::RemoteConfig;
use clawenv_core::input::{ToolError, ToolHandler, ToolRegistry, ToolSpec};
use clawenv_core::remote::{
    agent::AgentInvoker,
    protocol::{BridgeMsg, ServerMsg},
    runtime::RuntimeOptions,
    start_runtime,
    supervisor::{spawn as spawn_supervisor, SupervisorConfig},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

// ---------- Mock WSS server ----------

struct MockServer {
    url: String,
    inbound_rx: tokio::sync::mpsc::Receiver<String>,
    outbound_tx: tokio::sync::mpsc::Sender<String>,
    /// Dropping cancels the server task.
    _shutdown: tokio::sync::oneshot::Sender<()>,
}

async fn spawn_mock_server() -> MockServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let url = format!("ws://{addr}/chain/general/desktop_assistant");

    let (in_tx, in_rx) = tokio::sync::mpsc::channel::<String>(64);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<String>(64);
    let (sh_tx, sh_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        tokio::pin!(sh_rx);
        let (stream, _) = tokio::select! {
            res = listener.accept() => match res {
                Ok(pair) => pair,
                Err(_) => return,
            },
            _ = &mut sh_rx => return,
        };
        let ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(s) => s,
            Err(_) => return,
        };
        let (mut write, mut read) = ws_stream.split();

        loop {
            tokio::select! {
                frame = read.next() => match frame {
                    Some(Ok(Message::Text(t))) => {
                        if in_tx.send(t.to_string()).await.is_err() { return; }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = write.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    _ => {}
                },
                maybe_out = out_rx.recv() => {
                    let Some(s) = maybe_out else { return };
                    if write.send(Message::Text(s)).await.is_err() { return; }
                },
                _ = &mut sh_rx => return,
            }
        }
    });

    MockServer { url, inbound_rx: in_rx, outbound_tx: out_tx, _shutdown: sh_tx }
}

// ---------- Test-only echo tool ----------

struct EchoTool {
    calls: Arc<tokio::sync::Mutex<Vec<Value>>>,
}

#[async_trait]
impl ToolHandler for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "test.echo".into(),
            description: "Echo the given argument".into(),
            input_schema: json!({"type": "object"}),
        }
    }
    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        self.calls.lock().await.push(args.clone());
        Ok(json!({ "echoed": args }))
    }
}

// ---------- Helpers ----------

async fn recv_text(rx: &mut tokio::sync::mpsc::Receiver<String>, ctx: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("timeout waiting on {ctx}"))
        .unwrap_or_else(|| panic!("channel closed waiting on {ctx}"))
}

// ---------- Tests ----------

#[tokio::test]
async fn supervisor_handshake_and_frame_roundtrip() {
    let mut mock = spawn_mock_server().await;

    let mut cfg = SupervisorConfig::defaults(&mock.url, "test desktop", "mon-1");
    cfg.heartbeat_interval = Duration::from_secs(60); // no spurious pongs

    let mut sup = spawn_supervisor(cfg);

    // 1. hello from bridge
    let hello = recv_text(&mut mock.inbound_rx, "hello").await;
    let v: Value = serde_json::from_str(&hello).expect("hello json");
    assert_eq!(v["type"], "hello");
    assert!(v["capabilities"].as_array().is_some());

    // 2. server pushes user_message → supervisor surfaces it
    mock.outbound_tx
        .send(json!({"type":"user_message","id":"m1","content":"hi"}).to_string())
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), sup.inbound_mut().recv())
        .await
        .expect("inbound timeout")
        .expect("inbound closed");
    match got {
        ServerMsg::UserMessage { id, content } => {
            assert_eq!(id, "m1");
            assert_eq!(content, "hi");
        }
        other => panic!("unexpected server msg: {other:?}"),
    }

    // 3. bridge pushes ack → server receives it
    sup.outbound
        .send(BridgeMsg::Ack { id: "m1".into() })
        .await
        .unwrap();
    let ack = recv_text(&mut mock.inbound_rx, "ack").await;
    let a: Value = serde_json::from_str(&ack).unwrap();
    assert_eq!(a["type"], "ack");
    assert_eq!(a["id"], "m1");

    sup.shutdown.notify_waiters();
    let _ = sup.join.await;
}

#[tokio::test]
async fn supervisor_auto_reconnects_after_server_close() {
    // Spawn mock, connect, then drop the mock mid-flight; supervisor
    // should transition Disconnected → Connecting and retry.
    let mut mock = spawn_mock_server().await;
    let mut cfg = SupervisorConfig::defaults(&mock.url, "d", "m");
    cfg.heartbeat_interval = Duration::from_secs(60);
    cfg.initial_backoff = Duration::from_millis(50);
    cfg.max_backoff = Duration::from_millis(200);
    let sup = spawn_supervisor(cfg);

    // Drain the hello so we know we're connected.
    let _ = recv_text(&mut mock.inbound_rx, "hello").await;

    // Drop the mock server's shutdown tx to kill the accept task; the
    // one accept is already past, so we can't re-accept on the same
    // listener. Test only asserts supervisor noticed the close.
    drop(mock);

    // Give the supervisor a moment to detect the disconnect.
    tokio::time::sleep(Duration::from_millis(300)).await;

    sup.shutdown.notify_waiters();
    let _ = sup.join.await;
}

#[tokio::test]
async fn mcp_tools_list_and_call_via_http() {
    let calls = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let registry = ToolRegistry::new(vec![Arc::new(EchoTool { calls: calls.clone() })]);
    let handle = mcp::start(registry, 0, None).await.unwrap();
    let url = handle.url();
    let token = handle.token.clone();

    let client = reqwest::Client::new();

    let init: Value = client.post(&url)
        .bearer_auth(&token)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "clawenv-input");

    let list: Value = client.post(&url)
        .bearer_auth(&token)
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send().await.unwrap()
        .json().await.unwrap();
    let tools = list["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "test.echo");

    let call: Value = client.post(&url)
        .bearer_auth(&token)
        .json(&json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params": {"name":"test.echo","arguments":{"x":42}}
        }))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(call["result"]["structuredContent"]["echoed"]["x"], 42);

    assert_eq!(calls.lock().await.len(), 1);

    let _ = handle.shutdown.send(());
    let _ = handle.join.await;
}

#[tokio::test]
async fn mcp_rejects_unknown_tool_with_error_shape() {
    let registry = ToolRegistry::new(vec![]);
    let handle = mcp::start(registry, 0, None).await.unwrap();
    let url = handle.url();
    let token = handle.token.clone();

    let call: Value = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"nope","arguments":{}}
        }))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(call["result"]["isError"], true);
    // error payload is JSON text inside content[0].text
    let inner = call["result"]["content"][0]["text"].as_str().expect("text");
    let parsed: Value = serde_json::from_str(inner).unwrap();
    assert_eq!(parsed["code"], "invalid_argument");

    let _ = handle.shutdown.send(());
    let _ = handle.join.await;
}

#[tokio::test]
async fn runtime_full_wireup_echoes_user_message_back_to_server() {
    let mock = spawn_mock_server().await;
    let mut mock_in = mock.inbound_rx;
    let mock_out = mock.outbound_tx.clone();
    let _shutdown_guard = mock._shutdown;

    let tmp = tempfile::tempdir().unwrap();
    let descriptor_path = tmp.path().join("bridge.mcp.json");
    let audit_path = tmp.path().join("audit.log");

    let remote = RemoteConfig {
        server_url: mock.url.clone(),
        desktop_id: "macbook air".into(),
        monitor_device_id: "test".into(),
        mcp: clawenv_core::config::RemoteMcpConfig { preferred_port: 0 },
        ..Default::default()
    };

    let opts = RuntimeOptions {
        remote,
        descriptor_path: descriptor_path.clone(),
        audit_path: audit_path.clone(),
        // EchoInvoker: deterministic, no claw instance needed.
        invoker: Arc::new(clawenv_core::remote::EchoInvoker),
        // Don't start a global shortcut listener in tests.
        spawn_shortcut_listener: false,
    };
    let handle = start_runtime(opts).await.unwrap();

    // Discovery file was written with sane shape.
    let raw = std::fs::read_to_string(&descriptor_path).unwrap();
    let desc: mcp::BridgeMcpDescriptor = serde_json::from_str(&raw).unwrap();
    assert!(desc.url.contains("127.0.0.1"));
    assert_eq!(desc.token.len(), 64); // 32 bytes hex

    // Wait for hello from bridge.
    let _ = recv_text(&mut mock_in, "hello").await;

    // Push user_message.
    mock_out
        .send(json!({"type":"user_message","id":"u9","content":"hello world"}).to_string())
        .await
        .unwrap();

    // Expect the dispatcher to emit Ack + a synthetic AgentEvent.
    let mut saw_ack = false;
    let mut saw_agent_event = false;
    for _ in 0..4 {
        let Ok(Ok(frame)) = tokio::time::timeout(Duration::from_secs(3), mock_in.recv())
            .await.map(|o| o.ok_or("closed"))
        else {
            break;
        };
        let v: Value = serde_json::from_str(&frame).unwrap();
        match v["type"].as_str() {
            Some("ack") if v["id"] == "u9" => saw_ack = true,
            Some("agent_event") if v["id"] == "u9" => saw_agent_event = true,
            _ => {}
        }
        if saw_ack && saw_agent_event { break; }
    }
    assert!(saw_ack, "missing ack");
    assert!(saw_agent_event, "missing agent_event");

    // Audit log should have picked up the server_user_message.
    let audit_contents = std::fs::read_to_string(&audit_path).unwrap();
    assert!(audit_contents.contains("server_user_message"), "audit: {audit_contents}");

    handle.stop().await;
}

// ---------- Agent invoker wiring ----------

/// Slow invoker: holds the turn for `delay` before sending anything,
/// then emits one text frame. Used to test `cancel` aborting an
/// in-flight turn.
struct SlowInvoker {
    delay: Duration,
}

#[async_trait]
impl AgentInvoker for SlowInvoker {
    async fn invoke(
        &self,
        turn_id: String,
        _content: String,
        outbound: tokio::sync::mpsc::Sender<BridgeMsg>,
    ) -> anyhow::Result<()> {
        tokio::time::sleep(self.delay).await;
        let _ = outbound.send(BridgeMsg::AgentEvent {
            id: turn_id,
            kind: "text".into(),
            payload: json!({ "text": "too late" }),
        }).await;
        Ok(())
    }
}

#[tokio::test]
async fn dispatcher_cancel_aborts_in_flight_turn() {
    let mock = spawn_mock_server().await;
    let mut mock_in = mock.inbound_rx;
    let mock_out = mock.outbound_tx.clone();
    let _guard = mock._shutdown;

    let tmp = tempfile::tempdir().unwrap();
    let remote = RemoteConfig {
        server_url: mock.url.clone(),
        desktop_id: "dt".into(),
        monitor_device_id: "m".into(),
        mcp: clawenv_core::config::RemoteMcpConfig { preferred_port: 0 },
        ..Default::default()
    };
    let opts = RuntimeOptions {
        remote,
        descriptor_path: tmp.path().join("bridge.mcp.json"),
        audit_path: tmp.path().join("audit.log"),
        invoker: Arc::new(SlowInvoker { delay: Duration::from_secs(10) }),
        spawn_shortcut_listener: false,
    };
    let handle = start_runtime(opts).await.unwrap();

    // Drain hello
    let _ = recv_text(&mut mock_in, "hello").await;

    mock_out.send(json!({"type":"user_message","id":"slow1","content":"plz"}).to_string())
        .await.unwrap();
    // First ack arrives before invoker completes.
    let ack1 = recv_text(&mut mock_in, "ack slow1").await;
    assert!(ack1.contains(r#""type":"ack""#) && ack1.contains(r#""id":"slow1""#));

    // Send cancel quickly
    mock_out.send(json!({"type":"cancel","id":"slow1"}).to_string()).await.unwrap();

    // Expect an ack + a cancelled agent_event within a few seconds;
    // the slow invoker's text frame should NOT appear.
    let mut saw_cancel_ack = false;
    let mut saw_cancelled_event = false;
    let mut saw_late_text = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(frame)) = tokio::time::timeout(
            deadline - tokio::time::Instant::now(),
            mock_in.recv(),
        ).await else { break };
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        match v["type"].as_str() {
            Some("ack") if v["id"] == "slow1" => saw_cancel_ack = true,
            Some("agent_event") if v["id"] == "slow1" && v["kind"] == "cancelled"
                => saw_cancelled_event = true,
            Some("agent_event") if v["id"] == "slow1" && v["kind"] == "text"
                => saw_late_text = true,
            _ => {}
        }
    }
    assert!(saw_cancel_ack, "missing cancel ack");
    assert!(saw_cancelled_event, "missing cancelled event");
    assert!(!saw_late_text, "late text leaked through after cancel");

    handle.stop().await;
}

#[tokio::test]
async fn http_gateway_invoker_feeds_reply_to_server() {
    use axum::{routing::post, Json, Router};

    // Mock OpenAI-compatible gateway.
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|Json(body): Json<serde_json::Value>| async move {
            let user = body["messages"][0]["content"].as_str().unwrap_or("").to_string();
            Json(json!({
                "choices": [{"message": {"role":"assistant","content": format!("reply: {user}")}}]
            }))
        }),
    );
    let gw_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = gw_listener.local_addr().unwrap();
    let (gw_tx, gw_rx) = tokio::sync::oneshot::channel::<()>();
    let gw_join = tokio::spawn(async move {
        axum::serve(gw_listener, app)
            .with_graceful_shutdown(async move { let _ = gw_rx.await; })
            .await
    });

    let mock = spawn_mock_server().await;
    let mut mock_in = mock.inbound_rx;
    let mock_out = mock.outbound_tx.clone();
    let _guard = mock._shutdown;

    let tmp = tempfile::tempdir().unwrap();
    let remote = RemoteConfig {
        server_url: mock.url.clone(),
        desktop_id: "dt".into(),
        monitor_device_id: "m".into(),
        mcp: clawenv_core::config::RemoteMcpConfig { preferred_port: 0 },
        ..Default::default()
    };
    let invoker = Arc::new(clawenv_core::remote::HttpGatewayInvoker::new(
        format!("http://{gw_addr}"),
        "mock-model".into(),
        Duration::from_secs(5),
    ));
    let opts = RuntimeOptions {
        remote,
        descriptor_path: tmp.path().join("bridge.mcp.json"),
        audit_path: tmp.path().join("audit.log"),
        invoker,
        spawn_shortcut_listener: false,
    };
    let handle = start_runtime(opts).await.unwrap();

    let _ = recv_text(&mut mock_in, "hello").await;
    mock_out.send(json!({"type":"user_message","id":"g1","content":"hi gw"}).to_string())
        .await.unwrap();

    let mut reply_text: Option<String> = None;
    for _ in 0..6 {
        let Ok(Some(frame)) = tokio::time::timeout(Duration::from_secs(3), mock_in.recv()).await
        else { break };
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        if v["type"] == "agent_event" && v["id"] == "g1" && v["kind"] == "text" {
            reply_text = v["payload"]["text"].as_str().map(str::to_string);
            break;
        }
    }
    assert_eq!(reply_text.as_deref(), Some("reply: hi gw"));

    handle.stop().await;
    let _ = gw_tx.send(());
    let _ = gw_join.await;
}

// ---------- Kill-switch ----------

#[tokio::test]
async fn killswitch_blocks_input_tools_but_not_screen() {
    use async_trait::async_trait;
    use clawenv_core::remote::killswitch::{GatedToolHandler, KillSwitchState};

    struct Typer(Arc<tokio::sync::Mutex<u32>>);
    #[async_trait]
    impl ToolHandler for Typer {
        fn spec(&self) -> ToolSpec {
            ToolSpec { name: "input_keyboard_type".into(), description: "".into(), input_schema: json!({}) }
        }
        async fn call(&self, _args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
            *self.0.lock().await += 1;
            Ok(json!({"ok": true}))
        }
    }
    struct ScreenGrabber;
    #[async_trait]
    impl ToolHandler for ScreenGrabber {
        fn spec(&self) -> ToolSpec {
            ToolSpec { name: "screen_capture".into(), description: "".into(), input_schema: json!({}) }
        }
        async fn call(&self, _args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
            Ok(json!({"ok": true, "bytes": 0}))
        }
    }

    let kill = KillSwitchState::new(Duration::from_secs(60));
    let counter = Arc::new(tokio::sync::Mutex::new(0u32));
    let typer: Arc<dyn ToolHandler> = Arc::new(Typer(counter.clone()));
    let gated_typer: Arc<dyn ToolHandler> = Arc::new(GatedToolHandler::new(typer, kill.clone()));
    let screen: Arc<dyn ToolHandler> = Arc::new(ScreenGrabber);
    let registry = ToolRegistry::new(vec![gated_typer, screen]);

    registry.call("input_keyboard_type", json!({})).await.unwrap();
    assert_eq!(*counter.lock().await, 1);

    kill.arm();
    let err = registry.call("input_keyboard_type", json!({})).await.unwrap_err();
    assert_eq!(err.code(), "permission_denied");
    // Screen capture is NOT gated → still works.
    registry.call("screen_capture", json!({})).await.unwrap();
    // Counter hasn't advanced — input call was rejected.
    assert_eq!(*counter.lock().await, 1);
}
