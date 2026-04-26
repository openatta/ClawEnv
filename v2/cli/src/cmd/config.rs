//! `clawcli config <sub>` — read/write `~/.clawenv/config.toml`.
//!
//! CLI-DESIGN.md §2.4 + §7.17. Replaces v1's `settings save <json-blob>`
//! and v2-bridge's `settings save / diagnose` group with a clean
//! one-key-at-a-time surface.
//!
//! Keys use dot notation: `proxy.http`, `bridge.port`,
//! `mirrors.alpine_repo`, `clawenv.language`, `clawenv.theme`.

use clap::Subcommand;
use clawops_core::config_loader;

use crate::shared::Ctx;

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Dump the effective config (resolved global + env overlay).
    Show,
    /// Read one key (dot notation, e.g. `proxy.http`).
    Get { key: String },
    /// Write one key. Value parsed per the field's schema (string/
    /// bool/u16). Keychain-backed values (proxy.auth_password) get
    /// special-cased and stored in the OS credential vault.
    Set { key: String, value: String },
    /// Delete one key, reverting to its default.
    Unset { key: String },
    /// Parse + schema-check the file without writing. Returns ok or
    /// the first parse/schema error.
    Validate,
}

pub async fn run(cmd: ConfigCmd, ctx: &Ctx) -> anyhow::Result<()> {
    match cmd {
        ConfigCmd::Show => {
            // Emit the same dot-notation shape `get`/`set` use, so a
            // round-trip `show → set → show` is consistent. The nested
            // GlobalConfig is a Rust impl detail.
            let cfg = config_loader::load_global()
                .map_err(|e| anyhow::anyhow!("load: {e}"))?;
            let flat = flatten_config(&cfg);
            ctx.emit_pretty(&flat, |m| {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                for k in keys {
                    let v = m.get(k).map(|s| s.as_str()).unwrap_or("");
                    println!("{k} = {v}");
                }
            })?;
        }
        ConfigCmd::Get { key } => {
            let cfg = config_loader::load_global()
                .map_err(|e| anyhow::anyhow!("load: {e}"))?;
            let value = lookup_key(&cfg, &key)
                .ok_or_else(|| anyhow::anyhow!(
                    "config key `{key}` not known (try `clawcli config show`)"
                ))?;
            if ctx.json {
                ctx.output.emit(crate::output::CliEvent::Data {
                    data: serde_json::json!({"key": key, "value": value}),
                });
            } else {
                println!("{value}");
            }
        }
        ConfigCmd::Set { key, value } => {
            apply_set(&key, &value)
                .map_err(|e| anyhow::anyhow!("set `{key}`: {e}"))?;
            ctx.emit_text(format!("config: set {key} = {value}"));
        }
        ConfigCmd::Unset { key } => {
            apply_unset(&key)
                .map_err(|e| anyhow::anyhow!("unset `{key}`: {e}"))?;
            ctx.emit_text(format!("config: unset {key}"));
        }
        ConfigCmd::Validate => {
            // load_global() parses + applies serde defaults; success
            // here means the file is structurally OK.
            let _ = config_loader::load_global()
                .map_err(|e| anyhow::anyhow!("validate: {e}"))?;
            ctx.emit_text("config: ok");
        }
    }
    Ok(())
}

/// Build the flat `{dot.key: value}` view of GlobalConfig that
/// `config show` emits. Same key set as `lookup_key`, kept in sync —
/// add a key here whenever you add one there.
fn flatten_config(cfg: &config_loader::GlobalConfig) -> std::collections::BTreeMap<String, String> {
    let mut m = std::collections::BTreeMap::new();
    m.insert("proxy.enabled".into(), cfg.proxy.enabled.to_string());
    m.insert("proxy.http".into(), cfg.proxy.http_proxy.clone());
    m.insert("proxy.https".into(), cfg.proxy.https_proxy.clone());
    m.insert("proxy.no_proxy".into(), cfg.proxy.no_proxy.clone());
    m.insert("proxy.auth_required".into(), cfg.proxy.auth_required.to_string());
    m.insert("proxy.auth_user".into(), cfg.proxy.auth_user.clone());
    // proxy.auth_password is keychain-backed; never leaks here.
    m.insert("proxy.auth_password".into(), "(in keychain)".into());
    m.insert("mirrors.alpine_repo".into(), cfg.mirrors.alpine_repo.clone());
    m.insert("mirrors.npm_registry".into(), cfg.mirrors.npm_registry.clone());
    m.insert("bridge.enabled".into(), cfg.bridge.enabled.to_string());
    m.insert("bridge.port".into(), cfg.bridge.port.to_string());
    m
}

/// Resolve a dot-notation key against the loaded GlobalConfig. Returns
/// stringified value (so the caller doesn't have to type-switch).
fn lookup_key(cfg: &config_loader::GlobalConfig, key: &str) -> Option<String> {
    match key {
        "proxy.enabled"   => Some(cfg.proxy.enabled.to_string()),
        "proxy.http"      => Some(cfg.proxy.http_proxy.clone()),
        "proxy.https"     => Some(cfg.proxy.https_proxy.clone()),
        "proxy.no_proxy"  => Some(cfg.proxy.no_proxy.clone()),
        "proxy.auth_required" => Some(cfg.proxy.auth_required.to_string()),
        "proxy.auth_user" => Some(cfg.proxy.auth_user.clone()),
        // proxy.auth_password is keychain-backed — we never read it
        // back to the CLI (it'd surface in shell history). `set` writes
        // it to keychain; `get` returns "(in keychain)" sentinel.
        "proxy.auth_password" => Some("(in keychain)".into()),
        "mirrors.alpine_repo"  => Some(cfg.mirrors.alpine_repo.clone()),
        "mirrors.npm_registry" => Some(cfg.mirrors.npm_registry.clone()),
        "bridge.enabled" => Some(cfg.bridge.enabled.to_string()),
        "bridge.port"    => Some(cfg.bridge.port.to_string()),
        _ => None,
    }
}

fn apply_set(key: &str, value: &str) -> anyhow::Result<()> {
    use clawops_core::credentials;
    use clawops_core::proxy::ProxyConfig;
    match key {
        // Direct scalar fields under [clawenv].
        "language" | "theme" => {
            config_loader::save_clawenv_field(key, value)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        // Proxy section — load, mutate, save (atomic per section).
        "proxy.enabled" | "proxy.http" | "proxy.https" | "proxy.no_proxy"
        | "proxy.auth_required" | "proxy.auth_user" => {
            let mut p = config_loader::load_global()
                .map(|g| g.proxy).unwrap_or_default();
            match key {
                "proxy.enabled" => p.enabled = parse_bool(value)?,
                "proxy.http"    => p.http_proxy = value.to_string(),
                "proxy.https"   => p.https_proxy = value.to_string(),
                "proxy.no_proxy"=> p.no_proxy = value.to_string(),
                "proxy.auth_required" => p.auth_required = parse_bool(value)?,
                "proxy.auth_user"     => p.auth_user = value.to_string(),
                _ => unreachable!(),
            }
            config_loader::save_proxy_section(&p)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        "proxy.auth_password" => {
            // Keychain — never persisted to TOML.
            credentials::store_proxy_password(value)
                .map_err(|e| anyhow::anyhow!("keychain: {e}"))?;
        }
        // Bridge section — same pattern.
        "bridge.enabled" | "bridge.port" => {
            let mut b = config_loader::load_global()
                .map(|g| g.bridge).unwrap_or_default();
            match key {
                "bridge.enabled" => b.enabled = parse_bool(value)?,
                "bridge.port"    => b.port = value.parse()
                    .map_err(|e| anyhow::anyhow!("port must be u16: {e}"))?,
                _ => unreachable!(),
            }
            config_loader::save_bridge_section(&b)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        _ => anyhow::bail!(
            "unknown key `{key}` (try `clawcli config show` for the available set)"
        ),
    }
    let _ = ProxyConfig::default(); // ack import
    Ok(())
}

fn apply_unset(key: &str) -> anyhow::Result<()> {
    use clawops_core::credentials;
    match key {
        "proxy.auth_password" => {
            let _ = credentials::delete_proxy_password();
        }
        // For other keys: revert to default by re-saving the section
        // with the field cleared.
        "language" | "theme" => {
            config_loader::save_clawenv_field(key, "")
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        _ => anyhow::bail!(
            "unset of `{key}` not supported (only password-style keys are unsettable; \
             use `set <key> <default-value>` to revert other keys)"
        ),
    }
    Ok(())
}

fn parse_bool(s: &str) -> anyhow::Result<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on"  => Ok(true),
        "false"| "no"  | "0" | "off" => Ok(false),
        _ => anyhow::bail!("expected bool (true/false/yes/no/1/0/on/off), got `{s}`"),
    }
}
