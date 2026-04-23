//! One-shot WebSocket client for the remote channel.
//!
//! This layer handles *one* connection. Reconnect/backoff logic lives in
//! `supervisor.rs` on top. Keeping the two apart means tests can stand up
//! a single connection against a mock server without the reconnect loop
//! clouding assertions.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::protocol::{BridgeMsg, ServerMsg};

/// Characters that must NOT be percent-encoded in a URL query value.
/// Matches the "unreserved" set from RFC 3986 (ALPHA / DIGIT / `-` `.` `_` `~`).
/// Anything else — including space — is encoded. This gives `macbook air`
/// → `macbook%20air` (never `+`, which `form_urlencoded` would produce).
const QUERY_VALUE: &AsciiSet = &CONTROLS
    .add(b' ').add(b'"').add(b'#').add(b'%').add(b'&').add(b'+').add(b',')
    .add(b'/').add(b':').add(b';').add(b'<').add(b'=').add(b'>').add(b'?')
    .add(b'@').add(b'[').add(b'\\').add(b']').add(b'^').add(b'`').add(b'{')
    .add(b'|').add(b'}');

/// Compose the full WSS URL from a base endpoint + the two device ids.
pub fn build_url(base: &str, desktop_id: &str, monitor_device_id: &str) -> String {
    let d = utf8_percent_encode(desktop_id, QUERY_VALUE);
    let m = utf8_percent_encode(monitor_device_id, QUERY_VALUE);
    format!("{base}?desktop_id={d}&monitor_device_id={m}")
}

/// Live connection handle. Drop any half to trigger socket close.
pub struct WssConnection {
    pub inbound: mpsc::Receiver<ServerMsg>,
    pub outbound: mpsc::Sender<BridgeMsg>,
    pub join: tokio::task::JoinHandle<anyhow::Error>,
}

/// Open the socket and spawn the IO task. Returns a `WssConnection`; its
/// `join` handle completes with the reason the connection ended (network
/// error, server close, local drop). Caller uses that to decide reconnect.
pub async fn connect(url: &str) -> Result<WssConnection> {
    let (ws, _resp) = tokio_tungstenite::connect_async(url)
        .await
        .with_context(|| format!("ws connect failed: {url}"))?;
    let (mut write, mut read) = ws.split();

    let (in_tx, in_rx) = mpsc::channel::<ServerMsg>(64);
    let (out_tx, mut out_rx) = mpsc::channel::<BridgeMsg>(64);

    let join = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                maybe_frame = read.next() => match maybe_frame {
                    Some(Ok(Message::Text(t))) => match serde_json::from_str::<ServerMsg>(t.as_str()) {
                        Ok(msg) => {
                            if in_tx.send(msg).await.is_err() {
                                return anyhow::anyhow!("inbound receiver dropped");
                            }
                        }
                        Err(e) => tracing::warn!(target: "clawenv::remote", "bad server frame: {e} raw={t}"),
                    },
                    Some(Ok(Message::Binary(_))) => {
                        tracing::debug!(target: "clawenv::remote", "ignoring binary frame");
                    }
                    Some(Ok(Message::Ping(p))) => {
                        if let Err(e) = write.send(Message::Pong(p)).await {
                            return anyhow::anyhow!("pong write failed: {e}");
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => return anyhow::anyhow!("server closed socket"),
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(e)) => return anyhow::anyhow!("ws read error: {e}"),
                },
                maybe_out = out_rx.recv() => {
                    let Some(msg) = maybe_out else {
                        return anyhow::anyhow!("outbound sender dropped");
                    };
                    let text = match serde_json::to_string(&msg) {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!(target: "clawenv::remote", "serialize BridgeMsg: {e}");
                            continue;
                        }
                    };
                    if let Err(e) = write.send(Message::Text(text)).await {
                        return anyhow::anyhow!("ws write failed: {e}");
                    }
                }
            }
        }
    });

    Ok(WssConnection { inbound: in_rx, outbound: out_tx, join })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_url_encodes_space_as_percent20() {
        let u = build_url(
            "wss://api.example.com/x",
            "macbook air",
            "test",
        );
        assert_eq!(u, "wss://api.example.com/x?desktop_id=macbook%20air&monitor_device_id=test");
    }

    #[test]
    fn build_url_encodes_chinese() {
        let u = build_url("wss://h/x", "电脑", "屏一");
        // each CJK codepoint is 3 bytes in UTF-8, so we expect 3 %xx groups each.
        assert!(u.starts_with("wss://h/x?desktop_id="));
        assert!(u.contains("monitor_device_id="));
        assert!(!u.contains("电"));
    }
}
