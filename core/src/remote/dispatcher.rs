//! Routes `ServerMsg` frames into the local claw agent via an
//! `AgentInvoker`.
//!
//! Concurrency model:
//! - Each `user_message` registers a cancellation `Notify` in the
//!   `in_flight` map **before** the invoker task is spawned. This
//!   closes the observed race where a very fast invoker could complete
//!   (and try to remove its entry) before the spawning code inserted
//!   it, leaving a stale abort handle behind.
//! - The invoker runs inside `tokio::select!` against the Notify, so
//!   cancellation is cooperative — dropping the invoker future is how
//!   the turn actually stops. `AgentInvoker` implementations don't need
//!   to know about the token.
//! - `ack` goes out immediately on receipt, before the invoker starts,
//!   so the server sees fast acknowledgement independent of turn latency.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex, Notify};

use super::agent::AgentInvoker;
use super::audit::{AuditEvent, AuditLog};
use super::protocol::{BridgeMsg, ServerMsg};

pub struct Dispatcher {
    pub outbound: mpsc::Sender<BridgeMsg>,
    pub audit: Arc<AuditLog>,
    pub invoker: Arc<dyn AgentInvoker>,
}

type InFlight = Arc<Mutex<HashMap<String, Arc<Notify>>>>;

impl Dispatcher {
    pub async fn run(self, mut inbound: mpsc::Receiver<ServerMsg>) {
        let in_flight: InFlight = Arc::new(Mutex::new(HashMap::new()));

        while let Some(msg) = inbound.recv().await {
            match msg {
                ServerMsg::UserMessage { id, content } => {
                    self.audit.log(AuditEvent::ServerUserMessage {
                        id: id.clone(),
                        len: content.len(),
                    });
                    let _ = self
                        .outbound
                        .send(BridgeMsg::Ack { id: id.clone() })
                        .await;

                    // Insert BEFORE spawn so a very fast invoker can't race us.
                    let cancel = Arc::new(Notify::new());
                    in_flight.lock().await.insert(id.clone(), cancel.clone());

                    let invoker = self.invoker.clone();
                    let outbound = self.outbound.clone();
                    let audit = self.audit.clone();
                    let in_flight_task = in_flight.clone();
                    let turn_id = id;

                    tokio::spawn(async move {
                        let cancel_fut = cancel.notified();
                        tokio::pin!(cancel_fut);

                        tokio::select! {
                            biased;
                            _ = &mut cancel_fut => {
                                audit.log(AuditEvent::AgentCancelled {
                                    id: turn_id.clone(),
                                });
                                let _ = outbound
                                    .send(BridgeMsg::AgentEvent {
                                        id: turn_id.clone(),
                                        kind: "cancelled".into(),
                                        payload: serde_json::json!({}),
                                    })
                                    .await;
                            }
                            res = invoker.invoke(
                                turn_id.clone(),
                                content,
                                outbound.clone(),
                            ) => match res {
                                Ok(_) => audit.log(AuditEvent::AgentTurnComplete {
                                    id: turn_id.clone(),
                                }),
                                Err(e) => {
                                    let msg = e.to_string();
                                    audit.log(AuditEvent::AgentError {
                                        id: turn_id.clone(),
                                        message: msg.clone(),
                                    });
                                    let _ = outbound
                                        .send(BridgeMsg::AgentEvent {
                                            id: turn_id.clone(),
                                            kind: "error".into(),
                                            payload: serde_json::json!({ "message": msg }),
                                        })
                                        .await;
                                }
                            }
                        }

                        in_flight_task.lock().await.remove(&turn_id);
                    });
                }

                ServerMsg::Cancel { id } => {
                    self.audit.log(AuditEvent::ServerCancel { id: id.clone() });
                    let cancel = in_flight.lock().await.remove(&id);
                    let _ = self
                        .outbound
                        .send(BridgeMsg::Ack { id: id.clone() })
                        .await;
                    if let Some(n) = cancel {
                        // `notify_one` stores a permit if the in-flight
                        // task hasn't yet registered a waiter — crucial
                        // when a Cancel arrives tight on the heels of a
                        // UserMessage, before the spawned task has had
                        // its first poll. `notify_waiters` would drop
                        // that signal on the floor.
                        n.notify_one();
                    }
                    // If no matching id was found, we silently no-op:
                    // the server may be retrying, or the turn completed
                    // on its own before the cancel arrived.
                    let _ = id; // suppress unused in release builds
                }

                ServerMsg::Ping { .. } => {
                    // Supervisor short-circuits ping; ignore stragglers.
                }
                ServerMsg::Config { patch } => {
                    self.audit.log(AuditEvent::ServerConfig { patch });
                }
            }
        }

        // Inbound closed — cancel every running turn so invokers that
        // use spawn_blocking or hold network sockets get a chance to
        // tear down via the `select!` arm instead of being leaked.
        let mut map = in_flight.lock().await;
        for (_, n) in map.drain() {
            n.notify_one();
        }
        tracing::info!(target: "clawenv::remote", "dispatcher channel closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for the race fixed by switching to `notify_one`:
    /// calling `notify_one` BEFORE any waiter exists must still wake
    /// the next `notified().await`. With `notify_waiters` the signal
    /// would be dropped and the awaiter would hang forever.
    #[tokio::test]
    async fn notify_one_permit_survives_until_first_waiter() {
        let n = Arc::new(Notify::new());
        n.notify_one(); // No waiter yet.
        // `notified()` polled AFTER the notify must still resolve.
        tokio::time::timeout(std::time::Duration::from_millis(200), n.notified())
            .await
            .expect("notify_one's permit was dropped; cancel would hang");
    }
}
