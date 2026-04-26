//! GUI-only static descriptors for known claw products.
//!
//! v2 `clawops_core` deliberately keeps `ClawCli` lean (id, binary,
//! command-builders); GUI presentation fields like display name and
//! emoji logo live here so v2 core stays headless. Add a new arm when
//! a new claw product ships.

#[derive(Debug, Clone)]
pub struct ClawMeta {
    pub display_name: String,
    pub logo: String,
    /// npm registry package id (empty for non-npm claws).
    pub npm_package: &'static str,
}

pub fn meta_for(claw_id: &str) -> ClawMeta {
    match claw_id {
        "openclaw" => ClawMeta {
            display_name: "OpenClaw".into(),
            logo: "🐾".into(),
            npm_package: "@openatta/openclaw",
        },
        "hermes" => ClawMeta {
            display_name: "Hermes".into(),
            logo: "⚡".into(),
            npm_package: "",
        },
        other => ClawMeta {
            display_name: other.to_string(),
            logo: "📦".into(),
            npm_package: "",
        },
    }
}
