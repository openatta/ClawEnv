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
}
