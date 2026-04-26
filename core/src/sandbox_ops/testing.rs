//! Shared test helpers for sandbox_ops. Only compiled under cfg(test).
//!
//! Keeps a single, fully-instrumented `MockBackend` so each SandboxOps
//! impl can test its behavior against known backend responses without
//! touching real Lima/WSL/Podman.

#![cfg(test)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::provisioning::CreateOpts;
use crate::sandbox_backend::{ResourceStats, SandboxBackend};

/// Programmable fake backend.
///
/// - `start_err / stop_err` make start/stop fail
/// - `is_available_ret` toggles is_available() result
/// - `exec_responses` is a FIFO of (argv_shell_or_raw, result) pairs used
///   by exec; defaults to returning `canned_stdout` for any command
/// - `exec_log` records every raw cmd seen by `exec(&str)` in call order
pub(crate) struct MockBackend {
    pub name_str: &'static str,
    pub instance_str: String,
    pub start_calls: AtomicU32,
    pub stop_calls: AtomicU32,
    pub create_calls: AtomicU32,
    pub destroy_calls: AtomicU32,
    pub exec_log: Mutex<Vec<String>>,
    pub canned_stdout: Mutex<String>,
    /// Per-call scripted responses. Each `exec()` pops the front. When
    /// empty, falls back to `canned_stdout`. Lets tests drive a
    /// deterministic conversation with the mock (e.g. polling loops).
    pub scripted_responses: Mutex<VecDeque<String>>,
    pub start_err: bool,
    pub stop_err: bool,
    pub is_available_ret: bool,
    pub is_present_ret: bool,
}

impl MockBackend {
    pub fn new(name: &'static str) -> Self {
        Self {
            name_str: name,
            instance_str: "test".into(),
            start_calls: AtomicU32::new(0),
            stop_calls: AtomicU32::new(0),
            create_calls: AtomicU32::new(0),
            destroy_calls: AtomicU32::new(0),
            exec_log: Mutex::new(Vec::new()),
            canned_stdout: Mutex::new(String::new()),
            scripted_responses: Mutex::new(VecDeque::new()),
            start_err: false,
            stop_err: false,
            is_available_ret: true,
            is_present_ret: false,
        }
    }

    /// Enqueue a scripted response for the next `exec()` call.
    #[allow(dead_code)] // used by upcoming background tests
    pub fn queue_response(&self, s: impl Into<String>) {
        self.scripted_responses.lock().unwrap().push_back(s.into());
    }

    pub fn with_stdout(mut self, s: impl Into<String>) -> Self {
        *self.canned_stdout.get_mut().unwrap() = s.into();
        self
    }

    #[allow(dead_code)] // reserved for future doctor tests that toggle availability
    pub fn with_availability(mut self, avail: bool) -> Self {
        self.is_available_ret = avail;
        self
    }

    #[allow(dead_code)] // reserved for future repair tests
    pub fn with_start_err(mut self) -> Self {
        self.start_err = true;
        self
    }

    pub fn last_exec(&self) -> Option<String> {
        self.exec_log.lock().unwrap().last().cloned()
    }
}

#[async_trait]
impl SandboxBackend for MockBackend {
    fn name(&self) -> &str { self.name_str }
    fn instance(&self) -> &str { &self.instance_str }

    async fn is_available(&self) -> anyhow::Result<bool> {
        Ok(self.is_available_ret)
    }

    async fn is_present(&self) -> anyhow::Result<bool> {
        Ok(self.is_present_ret || self.is_available_ret)
    }

    async fn create(&self, _opts: &CreateOpts) -> anyhow::Result<()> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn destroy(&self) -> anyhow::Result<()> {
        self.destroy_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn start(&self) -> anyhow::Result<()> {
        self.start_calls.fetch_add(1, Ordering::SeqCst);
        if self.start_err { anyhow::bail!("mock start err"); }
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        self.stop_calls.fetch_add(1, Ordering::SeqCst);
        if self.stop_err { anyhow::bail!("mock stop err"); }
        Ok(())
    }

    async fn exec(&self, cmd: &str) -> anyhow::Result<String> {
        self.exec_log.lock().unwrap().push(cmd.to_string());
        if let Some(r) = self.scripted_responses.lock().unwrap().pop_front() {
            return Ok(r);
        }
        Ok(self.canned_stdout.lock().unwrap().clone())
    }

    async fn stats(&self) -> anyhow::Result<ResourceStats> {
        Ok(ResourceStats::default())
    }

    async fn edit_port_forwards(&self, _forwards: &[(u16, u16)]) -> anyhow::Result<()> {
        Ok(())
    }
}
