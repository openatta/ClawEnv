//! `clawcli bridge ...` — AttaRun bridge daemon config.
//!
//! Mirrors the Tauri IPCs `get_bridge_config` / `save_bridge_config`.
//! State lives in `~/.clawenv/config.toml` under `[clawenv.bridge]`.
//!
//! v2 didn't own config.toml writes before — the load path was
//! introduced in P1-a (read-only). This adds the matching write path
//! for the bridge subsection. Other config sections (proxy, mirrors)
//! still go through their own write surfaces.

use clap::Subcommand;
use clawops_core::config_loader;

use crate::shared::Ctx;

#[derive(Subcommand)]
pub enum BridgeCmd {
    /// Read the bridge-daemon config from `~/.clawenv/config.toml`.
    /// Returns the [clawenv.bridge] subsection as JSON; missing file
    /// or absent section both yield the default config (all defaults
    /// false / empty), so the GUI gets a stable shape.
    Config,
    /// Replace the bridge-daemon config. Reads a JSON blob on stdin
    /// (matching the shape `bridge config` returns) and writes it
    /// into [clawenv.bridge]. Other sections of config.toml are
    /// preserved verbatim.
    SaveConfig {
        /// Read JSON from stdin. Without --stdin we expect the JSON
        /// as the one positional arg (handy for one-liners).
        #[arg(long)]
        stdin: bool,
        /// Inline JSON when --stdin is not set.
        json: Option<String>,
    },
}

pub async fn run(cmd: BridgeCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        BridgeCmd::Config => {
            let cfg = config_loader::load_global()
                .map_err(|e| anyhow::anyhow!("load config: {e}"))?;
            ctx.emit(&cfg.bridge)?;
        }
        BridgeCmd::SaveConfig { stdin, json } => {
            let payload = if stdin {
                use std::io::Read;
                let mut s = String::new();
                std::io::stdin().read_to_string(&mut s)?;
                s
            } else {
                json.ok_or_else(|| anyhow::anyhow!(
                    "bridge save-config: pass JSON as positional arg or use --stdin"
                ))?
            };
            let parsed: clawops_core::config_loader::BridgeConfig =
                serde_json::from_str(&payload)
                    .map_err(|e| anyhow::anyhow!("parse bridge config json: {e}"))?;
            config_loader::save_bridge_section(&parsed)
                .map_err(|e| anyhow::anyhow!("save bridge config: {e}"))?;
            ctx.emit_text("bridge config saved");
        }
    }
    Ok(())
}
