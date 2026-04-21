//! Mock SandboxBackend for testing install/upgrade/instance flows.
//!
//! Records all exec() calls and returns pre-programmed responses.
//! This avoids needing real VMs for testing multi-claw command sequences.

#![cfg(test)]

use anyhow::Result;
use async_trait::async_trait;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::{ResourceStats, SandboxBackend, SandboxOpts};

/// A recorded exec() call.
#[derive(Debug, Clone)]
pub struct ExecCall {
    pub cmd: String,
}

/// A pre-programmed response for exec().
#[derive(Debug, Clone)]
pub struct ExecResponse {
    /// Pattern to match against the command (substring match)
    pub pattern: String,
    /// Response to return
    pub response: Result<String, String>,
}

/// Mock sandbox backend that records calls and returns programmed responses.
pub struct MockBackend {
    pub name: String,
    calls: Arc<Mutex<Vec<ExecCall>>>,
    responses: Arc<Mutex<VecDeque<ExecResponse>>>,
    /// Default response when no pattern matches
    default_response: String,
}

impl MockBackend {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            calls: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(VecDeque::new())),
            default_response: String::new(),
        }
    }

    /// Add a response: when exec() receives a command containing `pattern`,
    /// return `response`. Responses are consumed in order for matching patterns.
    pub fn on_exec(&mut self, pattern: &str, response: &str) -> &mut Self {
        self.responses.lock().unwrap().push_back(ExecResponse {
            pattern: pattern.into(),
            response: Ok(response.into()),
        });
        self
    }

    /// Add an error response for a pattern.
    pub fn on_exec_err(&mut self, pattern: &str, err: &str) -> &mut Self {
        self.responses.lock().unwrap().push_back(ExecResponse {
            pattern: pattern.into(),
            response: Err(err.into()),
        });
        self
    }

    /// Set the default response for unmatched commands.
    pub fn set_default_response(&mut self, response: &str) -> &mut Self {
        self.default_response = response.into();
        self
    }

    /// Get all recorded exec() calls.
    pub fn calls(&self) -> Vec<ExecCall> {
        self.calls.lock().unwrap().clone()
    }

    /// Get exec() calls that match a substring.
    pub fn calls_matching(&self, pattern: &str) -> Vec<ExecCall> {
        self.calls.lock().unwrap()
            .iter()
            .filter(|c| c.cmd.contains(pattern))
            .cloned()
            .collect()
    }

    /// Assert that at least one exec() call contained the given substring.
    pub fn assert_called_with(&self, pattern: &str) {
        let matches = self.calls_matching(pattern);
        assert!(
            !matches.is_empty(),
            "Expected exec() call containing '{}', but none found.\nAll calls:\n{}",
            pattern,
            self.calls().iter().map(|c| format!("  - {}", c.cmd)).collect::<Vec<_>>().join("\n")
        );
    }

    /// Assert no exec() call contained the given substring.
    pub fn assert_not_called_with(&self, pattern: &str) {
        let matches = self.calls_matching(pattern);
        assert!(
            matches.is_empty(),
            "Expected NO exec() call containing '{}', but found {} call(s)",
            pattern, matches.len()
        );
    }

    fn find_response(&self, cmd: &str) -> String {
        let mut responses = self.responses.lock().unwrap();
        // Find first matching response
        if let Some(pos) = responses.iter().position(|r| cmd.contains(&r.pattern)) {
            let resp = responses.remove(pos).unwrap();
            match resp.response {
                Ok(s) => return s,
                Err(e) => return format!("ERROR:{e}"),
            }
        }
        self.default_response.clone()
    }
}

#[async_trait]
impl SandboxBackend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn is_available(&self) -> Result<bool> {
        Ok(true)
    }

    async fn ensure_prerequisites(&self) -> Result<()> {
        Ok(())
    }

    async fn create(&self, _opts: &SandboxOpts) -> Result<()> {
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    async fn destroy(&self) -> Result<()> {
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> Result<String> {
        self.calls.lock().unwrap().push(ExecCall { cmd: cmd.into() });
        let response = self.find_response(cmd);
        if let Some(err) = response.strip_prefix("ERROR:") {
            anyhow::bail!("{}", err);
        }
        Ok(response)
    }

    async fn exec_with_progress(&self, cmd: &str, tx: &mpsc::Sender<String>) -> Result<String> {
        self.calls.lock().unwrap().push(ExecCall { cmd: cmd.into() });
        let response = self.find_response(cmd);
        let _ = tx.send(response.clone()).await;
        Ok(response)
    }

    async fn stats(&self) -> Result<ResourceStats> {
        Ok(ResourceStats::default())
    }

    async fn import_image(&self, _path: &Path) -> Result<()> {
        Ok(())
    }
}
