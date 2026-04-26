//! CLI output protocol — line-delimited JSON events.
//!
//! Ported verbatim from v1 `cli/src/output.rs` to keep the GUI's
//! `cli_bridge.rs` line-event parser compatible with v2 clawcli output.
//! Wire protocol (one event per stdout line in `--json` mode):
//!
//! ```text
//! {"type":"progress","stage":"...","percent":50,"message":"..."}
//! {"type":"info","message":"..."}
//! {"type":"data","data":<arbitrary JSON>}
//! {"type":"complete","message":"..."}
//! {"type":"error","message":"...","code":"..."}
//! ```
//!
//! The two coexist:
//! - **Streaming verbs** (install / upgrade / export) emit `progress`
//!   live and a final `data` + `complete`.
//! - **One-shot verbs** (list / status / fetch) emit a single `data`
//!   plus `complete`.
//!
//! G-migration plan (v2/docs/G-migration.md): every verb that the GUI
//! cares about should adopt this protocol so cli_bridge stays unchanged.
//!
//! G0 lands the protocol primitive. G1-a (this commit) wires Ctx + main
//! to emit through it.

use serde::Serialize;

/// A single CLI output event. Tagged-union JSON with snake_case `type`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CliEvent {
    Progress {
        stage: String,
        percent: u8,
        message: String,
    },
    Info {
        message: String,
    },
    Complete {
        message: String,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
    Data {
        data: serde_json::Value,
    },
}

/// Output handler — formats events for human or machine consumption.
#[derive(Clone)]
pub struct Output {
    json: bool,
}

impl Output {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    pub fn json(&self) -> bool { self.json }

    pub fn emit(&self, event: CliEvent) {
        if self.json {
            if let Ok(line) = serde_json::to_string(&event) {
                println!("{line}");
            }
        } else {
            self.emit_human(&event);
        }
    }

    fn emit_human(&self, event: &CliEvent) {
        match event {
            CliEvent::Progress { stage, percent, message } => {
                let bar = progress_bar(*percent);
                eprintln!("[{bar}] {percent:>3}% ({stage}) {message}");
            }
            CliEvent::Info { message } => {
                eprintln!("ℹ {message}");
            }
            CliEvent::Complete { message } => {
                eprintln!("✓ {message}");
            }
            CliEvent::Error { message, .. } => {
                eprintln!("✗ {message}");
            }
            CliEvent::Data { data } => {
                if let Ok(pretty) = serde_json::to_string_pretty(data) {
                    println!("{pretty}");
                }
            }
        }
    }
}

/// 20-char ASCII progress bar.
fn progress_bar(percent: u8) -> String {
    let filled = (percent as usize * 20) / 100;
    let empty = 20 - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_event_serializes_to_v1_wire() {
        let e = CliEvent::Progress {
            stage: "install".into(),
            percent: 50,
            message: "halfway".into(),
        };
        let s = serde_json::to_string(&e).unwrap();
        // Field order is deterministic in serde_json; sanity-check shape.
        assert!(s.contains(r#""type":"progress""#));
        assert!(s.contains(r#""stage":"install""#));
        assert!(s.contains(r#""percent":50"#));
    }

    #[test]
    fn data_event_wraps_arbitrary_value() {
        let e = CliEvent::Data {
            data: serde_json::json!({"foo": [1, 2, 3]}),
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""type":"data""#));
        assert!(s.contains(r#""foo":[1,2,3]"#));
    }

    #[test]
    fn error_omits_code_when_none() {
        let e = CliEvent::Error {
            message: "oops".into(),
            code: None,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(!s.contains("code"), "code field should be omitted: {s}");
    }

    #[test]
    fn error_includes_code_when_some() {
        let e = CliEvent::Error {
            message: "oops".into(),
            code: Some("E_FOO".into()),
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains(r#""code":"E_FOO""#), "missing code: {s}");
    }

    #[test]
    fn progress_bar_at_zero_is_empty() {
        let b = progress_bar(0);
        assert!(!b.contains('█'));
    }

    #[test]
    fn progress_bar_at_hundred_is_full() {
        let b = progress_bar(100);
        assert!(!b.contains('░'));
    }

    #[test]
    fn progress_bar_at_fifty_is_half() {
        let b = progress_bar(50);
        let filled = b.matches('█').count();
        let empty = b.matches('░').count();
        assert_eq!(filled, 10);
        assert_eq!(empty, 10);
    }

    #[test]
    fn output_json_mode_flag_is_queryable() {
        assert!(Output::new(true).json());
        assert!(!Output::new(false).json());
    }
}
