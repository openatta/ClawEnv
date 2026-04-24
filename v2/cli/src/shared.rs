//! Shared CLI helpers: context, output formatting.

use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Table};
use owo_colors::{OwoColorize, Stream};
use serde::Serialize;

pub struct Ctx {
    pub json: bool,
    pub quiet: bool,
    pub instance: String,
}

impl Ctx {
    /// JSON emission used by machine-consuming callers and as the fallback
    /// for non-json paths that haven't opted into a pretty printer yet.
    pub fn emit<T: Serialize + std::fmt::Debug>(&self, v: &T) -> anyhow::Result<()> {
        if self.json {
            let s = serde_json::to_string_pretty(v)?;
            println!("{s}");
        } else {
            println!("{v:#?}");
        }
        Ok(())
    }

    /// Pretty path: render a table/summary when stdout is a tty and `--json`
    /// is not set. `pretty` is only invoked in the human-readable case. In
    /// `--json` mode the value is emitted verbatim.
    pub fn emit_pretty<T, F>(&self, v: &T, pretty: F) -> anyhow::Result<()>
    where
        T: Serialize,
        F: FnOnce(&T),
    {
        if self.json {
            println!("{}", serde_json::to_string_pretty(v)?);
        } else {
            pretty(v);
        }
        Ok(())
    }

    pub fn emit_text(&self, s: impl AsRef<str>) {
        if !self.quiet {
            println!("{}", s.as_ref());
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

