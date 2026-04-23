//! Global kill-switch for the remote input MCP tools.
//!
//! Wire-level behaviour:
//! - A background thread listens to OS-level key events via `rdev`.
//! - When it observes the "nuke it" chord pressed three times within
//!   1.5s (`Cmd+Shift+Esc` on macOS, `Ctrl+Alt+Esc` on Windows), it
//!   writes the wall-clock "active-until" timestamp into a shared
//!   `AtomicI64`.
//! - Every MCP input tool call is wrapped in a `GatedToolHandler` which
//!   consults that timestamp; during the cooldown window the call
//!   returns `permission_denied`.
//!
//! The shared-state design has no channels and no async — an atomic
//! store/load is safe from the rdev thread and cheap per-MCP-call.

use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::remote::audit::{AuditEvent, AuditLog};

/// Cloneable handle to the kill-switch state.
///
/// `expires_at_unix` is the wall-clock second at which the cooldown
/// lifts (0 = not active). Using seconds keeps interop simple if the
/// value is later surfaced via status APIs.
#[derive(Clone)]
pub struct KillSwitchState {
    expires_at_unix: Arc<AtomicI64>,
    cooldown: Duration,
    /// Optional so tests can construct the state without a real log.
    audit: Option<Arc<AuditLog>>,
}

impl KillSwitchState {
    pub fn new(cooldown: Duration) -> Self {
        Self {
            expires_at_unix: Arc::new(AtomicI64::new(0)),
            cooldown,
            audit: None,
        }
    }

    pub fn with_audit(mut self, audit: Arc<AuditLog>) -> Self {
        self.audit = Some(audit);
        self
    }

    pub fn cooldown(&self) -> Duration {
        self.cooldown
    }

    pub fn is_active(&self) -> bool {
        let now = unix_now();
        let exp = self.expires_at_unix.load(Ordering::Acquire);
        exp > now
    }

    pub fn remaining(&self) -> Duration {
        let now = unix_now();
        let exp = self.expires_at_unix.load(Ordering::Acquire);
        if exp > now {
            Duration::from_secs((exp - now) as u64)
        } else {
            Duration::ZERO
        }
    }

    /// Arm (or extend) the cooldown window from now.
    pub fn arm(&self) {
        let exp = unix_now() + self.cooldown.as_secs() as i64;
        self.expires_at_unix.store(exp, Ordering::Release);
        tracing::warn!(
            target: "clawenv::remote",
            "kill-switch ARMED: remote input disabled for {}s",
            self.cooldown.as_secs()
        );
        if let Some(a) = &self.audit {
            a.log(AuditEvent::KillSwitchArmed {
                cooldown_sec: self.cooldown.as_secs(),
                expires_at_unix: exp,
            });
        }
    }

    /// Test-only: clear the cooldown.
    #[cfg(test)]
    pub fn disarm(&self) {
        self.expires_at_unix.store(0, Ordering::Release);
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ----------------------------------------------------------
// Global-shortcut listener (rdev-based). Spawned in its own
// OS thread; quietly no-ops on unsupported platforms.
// ----------------------------------------------------------

const MOD_CMD: u8 = 1 << 0;   // macOS Command
const MOD_CTRL: u8 = 1 << 1;  // Windows/Linux Control
const MOD_SHIFT: u8 = 1 << 2;
const MOD_ALT: u8 = 1 << 3;

fn required_mask() -> u8 {
    if cfg!(target_os = "macos") {
        MOD_CMD | MOD_SHIFT
    } else {
        MOD_CTRL | MOD_ALT
    }
}

/// Spawn the OS-level listener thread. Returns immediately; the thread
/// lives for the lifetime of the process (rdev has no clean stop API).
/// On listener init failure (usually macOS Accessibility not granted),
/// logs a warning and returns — the kill-switch simply won't trigger,
/// but the rest of the bridge keeps working.
///
/// Guarded by a process-wide `Once`: calling this multiple times
/// (eg. if the GUI toggles "enable remote" twice in one session) is
/// a no-op for every call after the first. The first-call `state` wins.
/// Subsequent calls with a *different* state would silently be ignored;
/// since that hasn't come up in practice, we only log a warning.
pub fn spawn_listener(state: KillSwitchState) {
    use std::sync::Once;
    static ONCE: Once = Once::new();

    // `is_completed()` is consistent with `call_once` — once the first
    // call finishes, subsequent calls see `true` and skip.
    if ONCE.is_completed() {
        tracing::warn!(
            target: "clawenv::remote",
            "kill-switch listener already running; ignoring duplicate spawn_listener call"
        );
        return;
    }
    ONCE.call_once(|| spawn_listener_inner(state));
}

fn spawn_listener_inner(state: KillSwitchState) {
    let mods = Arc::new(AtomicU8::new(0));

    std::thread::Builder::new()
        .name("clawenv-killswitch".into())
        .spawn(move || {
            // Bounded history of Escape-press timestamps.
            let mut history: Vec<Instant> = Vec::with_capacity(4);

            let cb_state = state.clone();
            let cb_mods = mods.clone();
            let result = rdev::listen(move |event| {
                use rdev::{EventType, Key};

                let bit = |k: &Key| -> Option<u8> {
                    match k {
                        Key::MetaLeft | Key::MetaRight => Some(MOD_CMD),
                        Key::ControlLeft | Key::ControlRight => Some(MOD_CTRL),
                        Key::ShiftLeft | Key::ShiftRight => Some(MOD_SHIFT),
                        Key::Alt | Key::AltGr => Some(MOD_ALT),
                        _ => None,
                    }
                };

                match &event.event_type {
                    EventType::KeyPress(k) => {
                        if let Some(b) = bit(k) {
                            cb_mods.fetch_or(b, Ordering::Relaxed);
                        } else if matches!(k, Key::Escape) {
                            let current = cb_mods.load(Ordering::Relaxed);
                            if current & required_mask() == required_mask() {
                                let now = Instant::now();
                                history.retain(|t| now.duration_since(*t) <= Duration::from_millis(1500));
                                history.push(now);
                                if history.len() >= 3 {
                                    cb_state.arm();
                                    history.clear();
                                }
                            }
                        }
                    }
                    EventType::KeyRelease(k) => {
                        if let Some(b) = bit(k) {
                            cb_mods.fetch_and(!b, Ordering::Relaxed);
                        }
                    }
                    _ => {}
                }
            });

            if let Err(e) = result {
                tracing::warn!(
                    target: "clawenv::remote",
                    "kill-switch listener failed to start ({e:?}); the chord will not fire. \
                     On macOS, grant Accessibility to this binary in System Settings."
                );
            }
        })
        .expect("spawn kill-switch thread");
}

// ----------------------------------------------------------
// Tool-handler gate
// ----------------------------------------------------------

use async_trait::async_trait;
use serde_json::Value;

use crate::input::{ToolError, ToolHandler, ToolSpec};

pub struct GatedToolHandler {
    inner: Arc<dyn ToolHandler>,
    state: KillSwitchState,
}

impl GatedToolHandler {
    pub fn new(inner: Arc<dyn ToolHandler>, state: KillSwitchState) -> Self {
        Self { inner, state }
    }
}

#[async_trait]
impl ToolHandler for GatedToolHandler {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }

    async fn call(&self, args: Value) -> Result<Value, ToolError> {
        if self.state.is_active() {
            let remaining = self.state.remaining().as_secs();
            return Err(ToolError::PermissionDenied(format!(
                "kill-switch active; cooldown {remaining}s remaining"
            )));
        }
        self.inner.call(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct Dummy;
    #[async_trait]
    impl ToolHandler for Dummy {
        fn spec(&self) -> ToolSpec {
            ToolSpec { name: "d".into(), description: "".into(), input_schema: json!({}) }
        }
        async fn call(&self, _args: Value) -> Result<Value, ToolError> {
            Ok(json!({ "ok": true }))
        }
    }

    #[tokio::test]
    async fn gate_passes_when_inactive_and_blocks_when_armed() {
        let state = KillSwitchState::new(Duration::from_secs(30));
        let gated = GatedToolHandler::new(Arc::new(Dummy), state.clone());

        assert_eq!(gated.call(json!({})).await.unwrap()["ok"], true);

        state.arm();
        let err = gated.call(json!({})).await.unwrap_err();
        assert_eq!(err.code(), "permission_denied");

        state.disarm();
        assert_eq!(gated.call(json!({})).await.unwrap()["ok"], true);
    }

    #[test]
    fn state_remaining_zero_when_inactive() {
        let s = KillSwitchState::new(Duration::from_secs(10));
        assert_eq!(s.remaining(), Duration::ZERO);
        assert!(!s.is_active());
    }
}
