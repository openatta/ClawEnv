//! Shared CLI helpers: context, output formatting.

use serde::Serialize;

pub struct Ctx {
    pub json: bool,
    pub quiet: bool,
    pub instance: String,
}

impl Ctx {
    /// Print a value in text or JSON form per `--json` flag.
    pub fn emit<T: Serialize + std::fmt::Debug>(&self, v: &T) -> anyhow::Result<()> {
        if self.json {
            let s = serde_json::to_string_pretty(v)?;
            println!("{}", s);
        } else {
            // Default text representation is debug-form; each command can
            // override with a prettier printer.
            println!("{:#?}", v);
        }
        Ok(())
    }

    pub fn emit_text(&self, s: impl AsRef<str>) {
        if !self.quiet {
            println!("{}", s.as_ref());
        }
    }
}
