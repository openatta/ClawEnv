use serde::{Deserialize, Serialize};

/// Wire-format version advertised in `BridgeMsg::Hello`. Bump when an
/// incompatible change lands in any of the typed frames below so the
/// server can refuse connections it can't parse rather than silently
/// mis-interpreting fields. Semver — server reads the major; minor
/// bumps are additive/optional.
pub const PROTOCOL_VERSION: &str = "0.1.0";

/// Messages sent by the remote server down to the bridge.
///
/// Tag discriminator is `type`. Unknown variants currently surface as a
/// parse error — the bridge logs and ignores them. If the server adds new
/// message types, extend this enum; never silently accept untyped bags.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// A new user turn. `content` is the raw user text; the bridge is
    /// expected to feed it into the local claw agent.
    UserMessage { id: String, content: String },
    /// Abort an in-flight turn.
    Cancel { id: String },
    /// Server-initiated keepalive. Bridge replies with `BridgeMsg::Pong`.
    Ping {
        #[serde(default)]
        ts: u64,
    },
    /// Out-of-band config patch (e.g. server pushes a new model id). Treated
    /// as opaque JSON; bridge decides which keys are applicable.
    Config { patch: serde_json::Value },
}

/// Messages sent by the bridge up to the server.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeMsg {
    /// First frame after socket handshake — announces bridge version,
    /// the wire-format version the bridge speaks, and the list of MCP
    /// tool names the local agent can invoke. `protocol_version` is
    /// the load-bearing one for server/bridge compatibility;
    /// `bridge_version` is informational (for telemetry).
    Hello {
        protocol_version: String,
        bridge_version: String,
        capabilities: Vec<String>,
    },
    /// Acknowledge receipt of a `UserMessage` / `Cancel`.
    Ack { id: String },
    /// Agent-emitted event correlated to a server turn. `kind` is one of
    /// `text` / `tool_call` / `tool_result` / `image` / `error`.
    AgentEvent {
        id: String,
        kind: String,
        payload: serde_json::Value,
    },
    /// Liveness / state-machine hint, unsolicited.
    Status {
        state: String,
        #[serde(default)]
        detail: String,
    },
    /// Reply to `ServerMsg::Ping`, or unprompted keepalive.
    Pong { ts: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_user_message_roundtrip() {
        let j = r#"{"type":"user_message","id":"m1","content":"hi"}"#;
        let msg: ServerMsg = serde_json::from_str(j).unwrap();
        match msg {
            ServerMsg::UserMessage { id, content } => {
                assert_eq!(id, "m1");
                assert_eq!(content, "hi");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn bridge_hello_serialises_with_type_tag() {
        let msg = BridgeMsg::Hello {
            protocol_version: PROTOCOL_VERSION.into(),
            bridge_version: "0.3.2".into(),
            capabilities: vec!["input.mouse".into()],
        };
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains(r#""type":"hello""#), "missing tag: {s}");
        assert!(s.contains(r#""bridge_version":"0.3.2""#));
        assert!(s.contains(r#""protocol_version":""#), "missing protocol_version: {s}");
    }

    #[test]
    fn server_ping_accepts_missing_ts() {
        let msg: ServerMsg = serde_json::from_str(r#"{"type":"ping"}"#).unwrap();
        matches!(msg, ServerMsg::Ping { ts: 0 });
    }
}
