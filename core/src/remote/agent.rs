//! Agent invocation — the "fed the user prompt into the claw instance"
//! half of the dispatcher.
//!
//! The trait is deliberately small: one method, `invoke`, which owns the
//! outbound channel and emits `BridgeMsg::AgentEvent` frames as the turn
//! unfolds. Each `invoke` call is a whole turn; the invoker is expected
//! to finish on its own or be cancelled via `tokio::task::AbortHandle`
//! from the dispatcher.
//!
//! Concrete impls:
//! - `HttpGatewayInvoker` talks to an OpenAI-compatible `/v1/chat/completions`
//!   endpoint. That's what OpenClaw's `gateway_port` serves.
//! - `EchoInvoker` — test/no-claw fallback that just synthesises a single
//!   `text` frame (`"[echo] ..."`). Used in Phase A and in tests.

use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::remote::protocol::BridgeMsg;

#[async_trait]
pub trait AgentInvoker: Send + Sync {
    /// Run one turn. Implementations emit zero or more
    /// `BridgeMsg::AgentEvent { id, kind, payload }` frames on `outbound`
    /// and return once the turn is complete. Propagate errors via `Err(_)`
    /// — the dispatcher will translate them into an `agent_event` with
    /// kind `error`.
    async fn invoke(
        &self,
        turn_id: String,
        content: String,
        outbound: mpsc::Sender<BridgeMsg>,
    ) -> anyhow::Result<()>;

    /// Short label for logs / audit (eg. "echo", "http_gateway",
    /// "slow_test"). Default is "custom".
    fn kind(&self) -> &'static str {
        "custom"
    }
}

// ------------------------------------------------------------
// EchoInvoker
// ------------------------------------------------------------

pub struct EchoInvoker;

#[async_trait]
impl AgentInvoker for EchoInvoker {
    fn kind(&self) -> &'static str { "echo" }

    async fn invoke(
        &self,
        turn_id: String,
        content: String,
        outbound: mpsc::Sender<BridgeMsg>,
    ) -> anyhow::Result<()> {
        let _ = outbound
            .send(BridgeMsg::AgentEvent {
                id: turn_id.clone(),
                kind: "text".into(),
                payload: serde_json::json!({ "text": format!("[echo] {content}") }),
            })
            .await;
        let _ = outbound
            .send(BridgeMsg::AgentEvent {
                id: turn_id,
                kind: "done".into(),
                payload: serde_json::json!({}),
            })
            .await;
        Ok(())
    }
}

// ------------------------------------------------------------
// HttpGatewayInvoker
// ------------------------------------------------------------

/// Minimal OpenAI-compatible chat client. Non-streaming for v0 — the
/// whole response is collected, then emitted as a single `text` event
/// followed by `done`. Streaming (SSE `text/event-stream`) is a
/// straightforward follow-up: switch to `client.post(...).send().await`
/// + `resp.bytes_stream()` + SSE line parser.
pub struct HttpGatewayInvoker {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
    pub client: reqwest::Client,
}

impl HttpGatewayInvoker {
    pub fn new(base_url: String, model: String, timeout: Duration) -> Self {
        Self {
            base_url,
            model,
            api_key: None,
            timeout,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_api_key(mut self, key: String) -> Self {
        self.api_key = Some(key);
        self
    }

    /// Pick a target claw instance from `AppConfig`.
    /// If `target` is non-empty, exact-match on instance name.
    /// Otherwise, first instance with `claw_type == "openclaw"`.
    /// Returns `None` if no candidate instance is configured.
    pub fn from_config(cfg: &AppConfig, target: &str, model: String, timeout: Duration) -> Option<Self> {
        let inst = if target.is_empty() {
            cfg.instances.iter().find(|i| i.claw_type == "openclaw")
        } else {
            cfg.instances.iter().find(|i| i.name == target)
        }?;
        let port = inst.gateway.gateway_port;
        Some(Self::new(format!("http://127.0.0.1:{port}"), model, timeout))
    }
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
}

#[async_trait]
impl AgentInvoker for HttpGatewayInvoker {
    fn kind(&self) -> &'static str { "http_gateway" }

    async fn invoke(
        &self,
        turn_id: String,
        content: String,
        outbound: mpsc::Sender<BridgeMsg>,
    ) -> anyhow::Result<()> {
        let body = ChatRequest {
            model: &self.model,
            messages: vec![ChatMessage { role: "user", content: &content }],
            stream: false,
        };
        let url = format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'));
        let mut req = self.client.post(&url).timeout(self.timeout).json(&body);
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }

        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("gateway returned HTTP {status}: {text}");
        }

        let json: serde_json::Value = resp.json().await?;
        // Hard-fail on unexpected shape: returning `""` to the server
        // would make the user see a blank agent reply with no signal of
        // what went wrong. A loud error in the audit log + agent_event
        // `error` frame is far more debuggable.
        let text = json
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!(
                "gateway response missing choices[0].message.content: {json}"
            ))?
            .to_string();

        // Guard against send failures: the supervisor might already be
        // tearing down. Dropping the send result is intentional.
        let _ = outbound
            .send(BridgeMsg::AgentEvent {
                id: turn_id.clone(),
                kind: "text".into(),
                payload: serde_json::json!({ "text": text }),
            })
            .await;
        let _ = outbound
            .send(BridgeMsg::AgentEvent {
                id: turn_id,
                kind: "done".into(),
                payload: serde_json::json!({
                    "model": self.model,
                    "bytes_in": content.len(),
                }),
            })
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::json;

    async fn spawn_mock_gateway(
        reply_text: &'static str,
    ) -> (String, tokio::task::JoinHandle<std::io::Result<()>>, tokio::sync::oneshot::Sender<()>)
    {
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(_body): Json<serde_json::Value>| async move {
                Json(json!({
                    "choices": [{"message": {"role":"assistant","content": reply_text}}]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let join = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move { let _ = rx.await; })
                .await
        });
        (format!("http://{addr}"), join, tx)
    }

    #[tokio::test]
    async fn http_invoker_round_trip() {
        let (url, join, sh) = spawn_mock_gateway("hello from mock").await;
        let invoker = HttpGatewayInvoker::new(url, "default".into(), Duration::from_secs(5));

        let (tx, mut rx) = mpsc::channel::<BridgeMsg>(8);
        invoker.invoke("t1".into(), "hi".into(), tx).await.unwrap();

        let first = rx.recv().await.unwrap();
        match first {
            BridgeMsg::AgentEvent { id, kind, payload } => {
                assert_eq!(id, "t1");
                assert_eq!(kind, "text");
                assert_eq!(payload["text"], "hello from mock");
            }
            other => panic!("unexpected {other:?}"),
        }
        let second = rx.recv().await.unwrap();
        match second {
            BridgeMsg::AgentEvent { kind, .. } => assert_eq!(kind, "done"),
            other => panic!("unexpected {other:?}"),
        }

        let _ = sh.send(());
        let _ = join.await;
    }

    #[tokio::test]
    async fn echo_invoker_emits_text_and_done() {
        let (tx, mut rx) = mpsc::channel::<BridgeMsg>(4);
        EchoInvoker.invoke("x".into(), "yo".into(), tx).await.unwrap();
        match rx.recv().await.unwrap() {
            BridgeMsg::AgentEvent { kind, payload, .. } => {
                assert_eq!(kind, "text");
                assert!(payload["text"].as_str().unwrap().contains("yo"));
            }
            other => panic!("{other:?}"),
        }
        match rx.recv().await.unwrap() {
            BridgeMsg::AgentEvent { kind, .. } => assert_eq!(kind, "done"),
            other => panic!("{other:?}"),
        }
    }
}
