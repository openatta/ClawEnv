//! Generic MCP-like tool registry used by the bridge MCP server.
//!
//! The trait is intentionally narrow — one method (`call`), one input
//! shape (`serde_json::Value`), one output shape. MCP's wire contract
//! (initialize / tools/list / tools/call) is layered on top by
//! `bridge::mcp`; this module knows nothing about JSON-RPC.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("unsupported on this platform: {0}")]
    Unsupported(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl ToolError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidArgument(_) => "invalid_argument",
            Self::PermissionDenied(_) => "permission_denied",
            Self::Unsupported(_) => "unsupported",
            Self::Internal(_) => "internal",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[async_trait]
pub trait ToolHandler: Send + Sync {
    fn spec(&self) -> ToolSpec;
    async fn call(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError>;
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    inner: Arc<HashMap<String, Arc<dyn ToolHandler>>>,
}

impl ToolRegistry {
    pub fn new(handlers: Vec<Arc<dyn ToolHandler>>) -> Self {
        let map: HashMap<_, _> = handlers
            .into_iter()
            .map(|h| (h.spec().name.clone(), h))
            .collect();
        Self { inner: Arc::new(map) }
    }

    pub fn list(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<_> = self.inner.values().map(|h| h.spec()).collect();
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        specs
    }

    pub async fn call(&self, name: &str, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let Some(handler) = self.inner.get(name) else {
            return Err(ToolError::InvalidArgument(format!("unknown tool '{name}'")));
        };
        handler.call(args).await
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<_> = self.inner.keys().cloned().collect();
        v.sort();
        v
    }

    /// Drop every handler whose name doesn't satisfy `keep`. Used by
    /// the GUI to enforce per-category opt-in: e.g. ship the keyboard
    /// handlers only when the user explicitly enabled keyboard control
    /// in `[clawenv.bridge.mcp]`.
    pub fn filter<F>(self, keep: F) -> Self
    where
        F: Fn(&str) -> bool,
    {
        let map: HashMap<String, Arc<dyn ToolHandler>> = self
            .inner
            .iter()
            .filter(|(name, _)| keep(name))
            .map(|(n, h)| (n.clone(), h.clone()))
            .collect();
        Self { inner: Arc::new(map) }
    }

    /// True when no handlers are registered. The bridge uses this to
    /// decide whether to expose the MCP routes / write the descriptor
    /// at all — there's no point advertising an empty server.
    pub fn is_empty(&self) -> bool { self.inner.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy;
    #[async_trait]
    impl ToolHandler for Dummy {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "x".into(),
                description: "".into(),
                input_schema: serde_json::json!({"type":"object"}),
            }
        }
        async fn call(&self, _args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
            Ok(serde_json::json!({"ok": true}))
        }
    }

    #[tokio::test]
    async fn registry_call_and_list() {
        let reg = ToolRegistry::new(vec![Arc::new(Dummy)]);
        assert_eq!(reg.names(), vec!["x".to_string()]);
        let out = reg.call("x", serde_json::json!({})).await.unwrap();
        assert_eq!(out["ok"], true);

        let err = reg.call("nope", serde_json::json!({})).await.unwrap_err();
        assert_eq!(err.code(), "invalid_argument");
    }

    struct Named(&'static str);
    #[async_trait]
    impl ToolHandler for Named {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: self.0.into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
            }
        }
        async fn call(&self, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
            Ok(serde_json::json!({}))
        }
    }

    #[test]
    fn filter_drops_handlers_outside_predicate() {
        let reg = ToolRegistry::new(vec![
            Arc::new(Named("input_keyboard_type")),
            Arc::new(Named("input_mouse_click")),
            Arc::new(Named("screen_capture")),
        ]);
        let filtered = reg.filter(|n| n.starts_with("screen_"));
        assert_eq!(filtered.names(), vec!["screen_capture".to_string()]);
        assert!(!filtered.is_empty());
    }

    #[test]
    fn empty_registry_is_empty() {
        let reg = ToolRegistry::new(vec![]);
        assert!(reg.is_empty());
    }
}
