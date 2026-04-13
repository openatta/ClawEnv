//! CLI output abstraction — human-readable or JSON lines.
//!
//! In JSON mode (`--json`), every event is a single JSON line on stdout.
//! GUI reads these lines to drive its UI. In human mode, events are
//! printed as formatted text with progress indicators.

use serde::Serialize;

/// A single CLI output event.
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
                // Pretty-print JSON data in human mode too
                if let Ok(pretty) = serde_json::to_string_pretty(data) {
                    println!("{pretty}");
                }
            }
        }
    }
}

/// Simple ASCII progress bar (20 chars wide).
fn progress_bar(percent: u8) -> String {
    let filled = (percent as usize * 20) / 100;
    let empty = 20 - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}
