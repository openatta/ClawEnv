//! Append-only JSONL audit log for every remote-initiated event.
//! Location: `~/.clawenv/remote-audit.log` (or custom path in tests).
//!
//! Event kinds are modelled as `AuditEvent` — a tagged enum serialised
//! as `{"event": "<variant_name>", ...fields}`. Centralising the names
//! here eliminates the "typo in a magic string" class of bug; the
//! compiler now enforces that every producer picks a known event.
//!
//! Two-level API:
//! - `AuditLog::log` takes an `AuditEvent` (the preferred path).
//! - `AuditLog::log_event` is the legacy untyped fallback kept for
//!   ad-hoc / migration use; new call sites should prefer the typed
//!   variant.
//!
//! Writes are lossy: a disk error is logged to tracing and dropped.
//! The goal is forensic trail, not durable messaging — losing a line
//! is less bad than stalling the MCP call that produced it.
//!
//! Rotation: when the file grows past `MAX_LOG_BYTES`, it is renamed
//! to `<path>.1` (overwriting any prior `.1`) and a fresh file is
//! started. Only two generations are ever kept — a daemon that runs
//! for months at low rate won't grow unbounded.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

/// Rotate when the live file exceeds this many bytes. Tuned so that a
/// daemon logging ~100 B per event can keep many tens of thousands of
/// events in the current generation, which is more than a typical
/// debugging window.
pub const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

/// Every event kind the remote runtime emits. Adding a new variant
/// forces every producer to compile; adding a field is source-compatible
/// with existing parsers because serde_json ignores extras by default.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AuditEvent {
    RuntimeStart,
    RuntimeStop,
    PermProbe {
        accessibility: String,
        screen_capture: String,
    },
    AgentMode {
        kind: &'static str,
    },
    ServerUserMessage {
        id: String,
        len: usize,
    },
    ServerCancel {
        id: String,
    },
    ServerConfig {
        patch: serde_json::Value,
    },
    AgentTurnComplete {
        id: String,
    },
    AgentCancelled {
        id: String,
    },
    AgentError {
        id: String,
        message: String,
    },
    KillSwitchArmed {
        cooldown_sec: u64,
        expires_at_unix: i64,
    },
    McpCall {
        tool: String,
    },
    /// Catch-all for the legacy untyped path. New code should prefer
    /// a typed variant; this exists so `log_event` still writes
    /// something sensible during migration.
    Untyped {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        msg_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<serde_json::Value>,
    },
}

/// Legacy untyped envelope kept around so producers that haven't been
/// migrated still write readable JSON. Once every call site goes
/// through `AuditEvent`, delete this and the `log_event` method.
#[derive(Debug, Clone, Serialize)]
pub struct RawAuditLine<'a> {
    pub ts: String,
    #[serde(flatten)]
    pub body: &'a AuditEvent,
}

pub struct AuditLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLog {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".clawenv")
            .join("remote-audit.log")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Preferred entry point. Emits a typed event.
    pub fn log(&self, event: AuditEvent) {
        let line = RawAuditLine {
            ts: chrono::Utc::now().to_rfc3339(),
            body: &event,
        };
        self.append_json(&line);
    }

    /// Legacy entry point. Packages the free-form args into
    /// `AuditEvent::Untyped` and delegates to `log`.
    pub fn log_event(
        &self,
        event: &str,
        msg_id: Option<String>,
        tool: Option<String>,
        detail: Option<serde_json::Value>,
    ) {
        self.log(AuditEvent::Untyped {
            name: event.to_string(),
            msg_id,
            tool,
            detail,
        });
    }

    fn append_json<T: Serialize>(&self, ev: &T) {
        let Ok(line) = serde_json::to_string(ev) else { return };
        let _guard = self.lock.lock();
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        self.rotate_if_needed();
        if let Err(e) = (|| -> std::io::Result<()> {
            let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
            writeln!(f, "{line}")?;
            Ok(())
        })() {
            tracing::warn!(target: "clawenv::remote", "audit log write failed: {e}");
        }
    }

    /// Size-capped rotation. Single `.1` generation. Called holding the
    /// internal lock. Failures are logged and swallowed — a rotation
    /// error should not stop the line write that triggered it, because
    /// the worst case is "log grows a bit past the cap", not data loss.
    fn rotate_if_needed(&self) {
        let Ok(meta) = std::fs::metadata(&self.path) else { return };
        if meta.len() < MAX_LOG_BYTES {
            return;
        }
        let rotated = {
            let mut s = self.path.as_os_str().to_os_string();
            s.push(".1");
            PathBuf::from(s)
        };
        // Remove any older `.1` first — `rename` on windows fails if
        // the destination exists.
        let _ = std::fs::remove_file(&rotated);
        if let Err(e) = std::fs::rename(&self.path, &rotated) {
            tracing::warn!(
                target: "clawenv::remote",
                "audit log rotation failed ({e}); continuing on oversized file"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn audit_writes_jsonl_line() {
        let dir = tempdir().unwrap();
        let log = AuditLog::open(dir.path().join("a.log"));
        log.log(AuditEvent::ServerUserMessage {
            id: "m1".into(),
            len: 42,
        });
        let contents = std::fs::read_to_string(dir.path().join("a.log")).unwrap();
        assert!(
            contents.contains(r#""event":"server_user_message""#),
            "got: {contents}"
        );
        assert!(contents.contains(r#""id":"m1""#));
        assert!(contents.contains(r#""len":42"#));
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn legacy_log_event_lands_as_untyped() {
        let dir = tempdir().unwrap();
        let log = AuditLog::open(dir.path().join("a.log"));
        log.log_event("something_else", Some("x".into()), None, None);
        let contents = std::fs::read_to_string(dir.path().join("a.log")).unwrap();
        assert!(contents.contains(r#""event":"untyped""#));
        assert!(contents.contains(r#""name":"something_else""#));
    }

    #[test]
    fn rotation_moves_file_past_size_cap() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("r.log");
        // Pre-fill past the cap.
        std::fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();
        let log = AuditLog::open(&path);
        log.log(AuditEvent::RuntimeStart);
        let rotated = path.with_file_name("r.log.1");
        assert!(rotated.exists(), "expected rotated file at {rotated:?}");
        let live = std::fs::read_to_string(&path).unwrap();
        assert!(live.contains("runtime_start"), "new line landed in fresh file: {live}");
        assert!(!live.starts_with("xxxx"), "rotation didn't reset the live file");
    }
}
