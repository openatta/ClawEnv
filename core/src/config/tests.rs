//! Config module tests: serialization, defaults, edge cases.

use super::*;

#[test]
fn minimal_toml_parses_with_defaults() {
    let toml = r#"
[clawenv]
version = "0.2.2"
"#;
    let config: AppConfig = toml::from_str(toml).unwrap();
    assert_eq!(config.clawenv.version, "0.2.2");
    assert_eq!(config.clawenv.language, "zh-CN");
    assert_eq!(config.clawenv.theme, "system");
    assert_eq!(config.clawenv.user_mode, UserMode::General);
    assert!(config.clawenv.updates.auto_check);
    assert_eq!(config.clawenv.updates.check_interval_hours, 24);
    assert!(config.instances.is_empty());
}

#[test]
fn instance_config_defaults() {
    let toml = r#"
[clawenv]
version = "0.2.2"

[[instances]]
name = "test"
version = "1.0.0"
sandbox_type = "lima-alpine"
created_at = "2026-01-01T00:00:00Z"
"#;
    let config: AppConfig = toml::from_str(toml).unwrap();
    let inst = &config.instances[0];
    assert_eq!(inst.name, "test");
    assert_eq!(inst.claw_type, "openclaw"); // default
    assert_eq!(inst.gateway.gateway_port, 3000); // default
    assert_eq!(inst.gateway.ttyd_port, 3001); // default
    assert_eq!(inst.resources.memory_limit_mb, 512); // default
    assert_eq!(inst.resources.cpu_cores, 2); // default
    assert!(!inst.browser.enabled); // default false
}

#[test]
fn hermes_instance_config() {
    let toml = r#"
[clawenv]
version = "0.2.2"

[[instances]]
name = "hermes-test"
claw_type = "hermes"
version = "Hermes Agent v0.9.0"
sandbox_type = "lima-alpine"
created_at = "2026-04-16T00:00:00Z"

[instances.gateway]
gateway_port = 3040
ttyd_port = 3041
"#;
    let config: AppConfig = toml::from_str(toml).unwrap();
    let inst = &config.instances[0];
    assert_eq!(inst.claw_type, "hermes");
    assert_eq!(inst.gateway.gateway_port, 3040);
    assert_eq!(inst.gateway.ttyd_port, 3041);
}

#[test]
fn config_roundtrip_toml() {
    let original = AppConfig {
        clawenv: ClawEnvConfig {
            version: "0.2.2".into(),
            user_mode: UserMode::Developer,
            language: "en".into(),
            theme: "dark".into(),
            updates: UpdateConfig::default(),
            security: SecurityConfig::default(),
            tray: TrayConfig::default(),
            proxy: ProxyConfig::default(),
            mirrors: MirrorsConfig::default(),
            bridge: BridgeConfig::default(),
        },
        instances: vec![],
    };
    let serialized = toml::to_string(&original).unwrap();
    let deserialized: AppConfig = toml::from_str(&serialized).unwrap();
    assert_eq!(deserialized.clawenv.version, "0.2.2");
    assert_eq!(deserialized.clawenv.user_mode, UserMode::Developer);
    assert_eq!(deserialized.clawenv.language, "en");
}

#[test]
fn mirrors_default_preset() {
    let m = MirrorsConfig::default();
    assert!(m.is_default());
    assert_eq!(m.alpine_repo_url(), "https://dl-cdn.alpinelinux.org/alpine");
    assert_eq!(m.npm_registry_url(), "https://registry.npmjs.org");
    assert_eq!(m.nodejs_dist_url(), "https://nodejs.org/dist");
}

#[test]
fn mirrors_china_preset() {
    let m = MirrorsConfig {
        preset: "china".into(),
        ..Default::default()
    };
    assert!(!m.is_default());
    assert_eq!(m.alpine_repo_url(), "https://mirrors.aliyun.com/alpine");
    assert_eq!(m.npm_registry_url(), "https://registry.npmmirror.com");
}

#[test]
fn mirrors_custom_overrides_preset() {
    let m = MirrorsConfig {
        preset: "china".into(),
        npm_registry: "https://my.custom.registry".into(),
        ..Default::default()
    };
    // Custom URL overrides preset
    assert_eq!(m.npm_registry_url(), "https://my.custom.registry");
    // But alpine still uses china preset
    assert_eq!(m.alpine_repo_url(), "https://mirrors.aliyun.com/alpine");
}

#[test]
fn proxy_config_defaults() {
    let p = ProxyConfig::default();
    assert!(!p.enabled);
    assert!(p.http_proxy.is_empty());
    assert_eq!(p.no_proxy, "localhost,127.0.0.1");
}

#[test]
fn user_mode_serialization() {
    let general = serde_json::to_string(&UserMode::General).unwrap();
    assert_eq!(general, "\"general\"");
    let developer = serde_json::to_string(&UserMode::Developer).unwrap();
    assert_eq!(developer, "\"developer\"");
    // Deserialize
    let parsed: UserMode = serde_json::from_str("\"developer\"").unwrap();
    assert_eq!(parsed, UserMode::Developer);
}
