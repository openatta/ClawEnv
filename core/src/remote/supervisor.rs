//! Reconnect + heartbeat wrapper around `wss_client`.
//!
//! External view: call `spawn(cfg)` once, get back channels for inbound
//! `ServerMsg` / outbound `BridgeMsg`, plus a status watcher. The supervisor
//! takes care of:
//!   - exponential backoff reconnect (1s → 60s cap)
//!   - 30s heartbeat (outbound Pong) if the server goes quiet
//!   - 60s read-idle timeout → force reconnect
//!   - graceful shutdown via the returned `Notify`

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch, Notify};
use tokio::time::{sleep, Instant};

use super::protocol::{BridgeMsg, ServerMsg, PROTOCOL_VERSION};
use super::wss_client;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected,
    Disconnected,
}

pub struct SupervisorConfig {
    pub server_url: String,
    pub desktop_id: String,
    pub monitor_device_id: String,
    pub heartbeat_interval: Duration,
    pub read_idle_timeout: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub capabilities: Vec<String>,
}

impl SupervisorConfig {
    pub fn defaults(server_url: impl Into<String>, desktop_id: impl Into<String>, monitor_device_id: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            desktop_id: desktop_id.into(),
            monitor_device_id: monitor_device_id.into(),
            heartbeat_interval: Duration::from_secs(30),
            read_idle_timeout: Duration::from_secs(60),
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            capabilities: vec![
                "input.keyboard".into(),
                "input.mouse".into(),
                "screen.capture".into(),
            ],
        }
    }
}

pub struct SupervisorHandle {
    /// `Option` so that `take_inbound()` can move out cleanly. Direct
    /// read access via `inbound_mut()` for callers that hold the handle
    /// (eg. CLI `test-connect`) and want to drain without taking.
    inbound: Option<mpsc::Receiver<ServerMsg>>,
    pub outbound: mpsc::Sender<BridgeMsg>,
    pub status: watch::Receiver<Status>,
    pub shutdown: Arc<Notify>,
    pub join: tokio::task::JoinHandle<()>,
}

impl SupervisorHandle {
    /// Move the inbound receiver out — exactly once. Subsequent calls
    /// return `None`. Use this when handing ownership to a dispatcher.
    pub fn take_inbound(&mut self) -> Option<mpsc::Receiver<ServerMsg>> {
        self.inbound.take()
    }

    /// Borrow the inbound receiver mutably for direct `.recv()` calls.
    /// Panics if `take_inbound()` has already consumed it — callers that
    /// mix the two are doing something wrong.
    pub fn inbound_mut(&mut self) -> &mut mpsc::Receiver<ServerMsg> {
        self.inbound.as_mut().expect("inbound already taken")
    }
}

pub fn spawn(cfg: SupervisorConfig) -> SupervisorHandle {
    let (in_tx, in_rx) = mpsc::channel::<ServerMsg>(64);
    let (out_tx, mut out_rx) = mpsc::channel::<BridgeMsg>(64);
    let (status_tx, status_rx) = watch::channel(Status::Connecting);
    let shutdown = Arc::new(Notify::new());
    let shutdown_task = shutdown.clone();

    let join = tokio::spawn(async move {
        let mut backoff = cfg.initial_backoff;
        'outer: loop {
            status_tx.send(Status::Connecting).ok();
            let url = wss_client::build_url(&cfg.server_url, &cfg.desktop_id, &cfg.monitor_device_id);
            tracing::info!(target: "clawenv::remote", "connecting to {url}");

            let mut conn = tokio::select! {
                biased;
                _ = shutdown_task.notified() => break 'outer,
                res = wss_client::connect(&url) => match res {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(target: "clawenv::remote", "connect error: {e}");
                        status_tx.send(Status::Disconnected).ok();
                        tokio::select! {
                            biased;
                            _ = shutdown_task.notified() => break 'outer,
                            _ = sleep(backoff) => {}
                        }
                        backoff = (backoff * 2).min(cfg.max_backoff);
                        continue 'outer;
                    }
                }
            };

            status_tx.send(Status::Connected).ok();
            backoff = cfg.initial_backoff;

            // First frame out: hello.
            let _ = conn
                .outbound
                .send(BridgeMsg::Hello {
                    protocol_version: PROTOCOL_VERSION.to_string(),
                    bridge_version: env!("CARGO_PKG_VERSION").to_string(),
                    capabilities: cfg.capabilities.clone(),
                })
                .await;

            let mut last_recv = Instant::now();
            let mut heartbeat = tokio::time::interval(cfg.heartbeat_interval);
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            heartbeat.tick().await; // drain immediate first tick

            'inner: loop {
                tokio::select! {
                    biased;
                    _ = shutdown_task.notified() => break 'outer,
                    maybe_srv = conn.inbound.recv() => {
                        let Some(msg) = maybe_srv else { break 'inner };
                        last_recv = Instant::now();
                        match msg {
                            ServerMsg::Ping { ts } => {
                                let _ = conn.outbound.send(BridgeMsg::Pong { ts }).await;
                            }
                            other => {
                                if in_tx.send(other).await.is_err() { break 'outer; }
                            }
                        }
                    }
                    maybe_out = out_rx.recv() => {
                        let Some(msg) = maybe_out else { break 'outer };
                        if conn.outbound.send(msg).await.is_err() { break 'inner; }
                    }
                    _ = heartbeat.tick() => {
                        if last_recv.elapsed() > cfg.read_idle_timeout {
                            tracing::warn!(target: "clawenv::remote", "read idle > {:?}, reconnecting", cfg.read_idle_timeout);
                            break 'inner;
                        }
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let _ = conn.outbound.send(BridgeMsg::Pong { ts }).await;
                    }
                }
            }

            // Inner loop exited → tear down this connection, back off, retry.
            drop(conn);
            status_tx.send(Status::Disconnected).ok();
            tracing::info!(target: "clawenv::remote", "disconnected, retry in {:?}", backoff);
            tokio::select! {
                biased;
                _ = shutdown_task.notified() => break 'outer,
                _ = sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(cfg.max_backoff);
        }

        status_tx.send(Status::Disconnected).ok();
        tracing::info!(target: "clawenv::remote", "supervisor stopped");
    });

    SupervisorHandle {
        inbound: Some(in_rx),
        outbound: out_tx,
        status: status_rx,
        shutdown,
        join,
    }
}
