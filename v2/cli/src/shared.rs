//! Shared CLI helpers: context, output formatting.
//!
//! `--json` mode now emits **line-delimited CliEvents** (the v1 wire
//! protocol the GUI's cli_bridge.rs expects). This module's `Ctx`
//! routes every value through `Output::emit(CliEvent::Data {...})`
//! so individual verbs don't have to know about the protocol.

use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Table};
use owo_colors::{OwoColorize, Stream};
use serde::Serialize;

use crate::output::{CliEvent, Output};

pub struct Ctx {
    pub json: bool,
    pub quiet: bool,
    pub instance: String,
    /// Wire-protocol emitter. Cheap to clone — it's just a `bool`.
    pub output: Output,
}

impl Ctx {
    /// Emit `v` as a `Data` CliEvent in --json mode, or as Rust Debug
    /// in human mode (fallback for verbs that haven't opted into a
    /// pretty printer).
    pub fn emit<T: Serialize + std::fmt::Debug>(&self, v: &T) -> anyhow::Result<()> {
        if self.json {
            let value = serde_json::to_value(v)?;
            self.output.emit(CliEvent::Data { data: value });
        } else {
            println!("{v:#?}");
        }
        Ok(())
    }

    /// Pretty path: render a table/summary in human mode, emit a Data
    /// event in --json mode. `pretty` is only called when human.
    pub fn emit_pretty<T, F>(&self, v: &T, pretty: F) -> anyhow::Result<()>
    where
        T: Serialize,
        F: FnOnce(&T),
    {
        if self.json {
            let value = serde_json::to_value(v)?;
            self.output.emit(CliEvent::Data { data: value });
        } else {
            pretty(v);
        }
        Ok(())
    }

    /// Emit a free-text `Info` line. In --json mode this becomes
    /// `{"type":"info","message":"..."}`; in human mode just println.
    pub fn emit_text(&self, s: impl AsRef<str>) {
        if self.quiet {
            return;
        }
        let msg = s.as_ref().to_string();
        if self.json {
            self.output.emit(CliEvent::Info { message: msg });
        } else {
            println!("{msg}");
        }
    }
}

/// Build a comfy-table pre-styled for ClawEnv CLI output.
pub fn new_table<I, S>(headers: I) -> Table
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut t = Table::new();
    t.load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.into_iter().map(|h| Cell::new(h.into())));
    t
}

/// Color a severity string green/yellow/red by level. No-op when stdout
/// isn't a tty so pipes stay ANSI-free (also what owo-colors' `if_supports_color`
/// does under the hood, but being explicit makes the intent obvious).
pub fn severity_color(level: &str) -> String {
    match level {
        "error" | "Error" => level
            .if_supports_color(Stream::Stdout, |s| s.red().bold().to_string())
            .to_string(),
        "warning" | "Warning" => level
            .if_supports_color(Stream::Stdout, |s| s.yellow().bold().to_string())
            .to_string(),
        "info" | "Info" => level
            .if_supports_color(Stream::Stdout, |s| s.cyan().to_string())
            .to_string(),
        _ => level.to_string(),
    }
}

