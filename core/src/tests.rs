#[cfg(test)]
mod tests {
    // ===== Platform Detection =====

    #[test]
    fn test_platform_detection() {
        let platform = crate::platform::detect_platform().expect("Platform detection failed");
        assert!(matches!(
            platform.os,
            crate::platform::OsType::Windows
                | crate::platform::OsType::Macos
                | crate::platform::OsType::Linux
        ));
        assert!(matches!(
            platform.arch,
            crate::platform::Arch::X86_64 | crate::platform::Arch::Aarch64
        ));
    }

    // ===== Instance Name Validation =====

    #[test]
    fn test_instance_name_valid() {
        use crate::manager::install::validate_instance_name;
        assert!(validate_instance_name("default").is_ok());
        assert!(validate_instance_name("my-instance").is_ok());
        assert!(validate_instance_name("test_123").is_ok());
        assert!(validate_instance_name("a").is_ok());
        assert!(validate_instance_name(&"a".repeat(63)).is_ok());
    }

    #[test]
    fn test_instance_name_invalid() {
        use crate::manager::install::validate_instance_name;
        assert!(validate_instance_name("").is_err());
        assert!(validate_instance_name(&"a".repeat(64)).is_err());
        assert!(validate_instance_name("../../../etc").is_err());
        assert!(validate_instance_name("$(whoami)").is_err());
        assert!(validate_instance_name("-invalid").is_err());
        assert!(validate_instance_name("has space").is_err());
        assert!(validate_instance_name("has@symbol").is_err());
        assert!(validate_instance_name("über").is_err());
    }

    // ===== Shell Escaping =====

    #[test]
    fn test_shell_escape_passthrough() {
        use crate::manager::install::shell_escape;
        assert_eq!(shell_escape("simple"), "simple");
        assert_eq!(shell_escape("abc123"), "abc123");
        assert_eq!(shell_escape("sk-abc-123-xyz"), "sk-abc-123-xyz");
    }

    #[test]
    fn test_shell_escape_single_quotes() {
        use crate::manager::install::shell_escape;
        assert_eq!(shell_escape("it's"), "it'\\''s");
        assert_eq!(shell_escape("'"), "'\\''");
        assert_eq!(shell_escape("a'b'c"), "a'\\''b'\\''c");
    }

    #[test]
    fn test_shell_escape_preserves_other_chars() {
        use crate::manager::install::shell_escape;
        // Inside single quotes, these are safe — shell_escape only handles single quotes
        assert_eq!(shell_escape("$(cmd)"), "$(cmd)");
        assert_eq!(shell_escape("a\"b"), "a\"b");
        assert_eq!(shell_escape("a\\b"), "a\\b");
        assert_eq!(shell_escape("a\nb"), "a\nb");
    }

    // ===== Config Models =====

    #[test]
    fn test_config_defaults() {
        let config = crate::config::AppConfig {
            clawenv: crate::config::ClawEnvConfig {
                version: "0.1.0".into(),
                user_mode: crate::config::UserMode::General,
                language: "zh-CN".into(),
                theme: "system".into(),
                updates: Default::default(),
                security: Default::default(),
                tray: Default::default(),
                proxy: Default::default(),
            },
            instances: vec![],
        };
        assert_eq!(config.clawenv.tray.monitor_interval_sec, 5);
        assert!(config.clawenv.tray.enabled);
        assert!(!config.clawenv.tray.start_minimized);
        assert_eq!(config.clawenv.proxy.no_proxy, "localhost,127.0.0.1");
        assert!(!config.clawenv.proxy.enabled);
        assert!(config.clawenv.updates.auto_check);
        assert_eq!(config.clawenv.updates.snapshot_retention_count, 5);
    }

    #[test]
    fn test_config_toml_roundtrip() {
        let config = crate::config::AppConfig {
            clawenv: crate::config::ClawEnvConfig {
                version: "0.1.0".into(),
                user_mode: crate::config::UserMode::Developer,
                language: "en".into(),
                theme: "dark".into(),
                updates: Default::default(),
                security: Default::default(),
                tray: crate::config::TrayConfig {
                    enabled: false,
                    start_minimized: true,
                    show_notifications: false,
                    monitor_interval_sec: 10,
                },
                proxy: crate::config::ProxyConfig {
                    enabled: true,
                    http_proxy: "http://proxy:8080".into(),
                    https_proxy: "".into(),
                    no_proxy: "localhost".into(),
                    auth_required: false,
                    auth_user: "".into(),
                },
            },
            instances: vec![],
        };

        let toml_str = toml::to_string_pretty(&config).expect("Serialize failed");
        let parsed: crate::config::AppConfig =
            toml::from_str(&toml_str).expect("Deserialize failed");

        assert_eq!(parsed.clawenv.theme, "dark");
        assert_eq!(parsed.clawenv.user_mode, crate::config::UserMode::Developer);
        assert!(!parsed.clawenv.tray.enabled);
        assert_eq!(parsed.clawenv.tray.monitor_interval_sec, 10);
        assert!(parsed.clawenv.proxy.enabled);
        assert_eq!(parsed.clawenv.proxy.http_proxy, "http://proxy:8080");
    }

    #[test]
    fn test_config_toml_with_instance() {
        let toml_str = r#"
[clawenv]
version = "0.1.0"
user_mode = "general"
language = "zh-CN"
theme = "system"

[[instances]]
name = "default"
claw_type = "openclaw"
version = "2.1.3"
sandbox_type = "lima-alpine"
sandbox_id = "clawenv-default"
created_at = "2026-04-01T10:00:00Z"
"#;
        let config: crate::config::AppConfig =
            toml::from_str(toml_str).expect("Parse failed");
        assert_eq!(config.instances.len(), 1);
        assert_eq!(config.instances[0].name, "default");
        assert_eq!(
            config.instances[0].sandbox_type,
            crate::sandbox::SandboxType::LimaAlpine
        );
    }

    // ===== Version Comparison =====

    #[test]
    fn test_version_upgrade_available() {
        use crate::update::checker::VersionInfo;
        let info = VersionInfo {
            current: "2.1.3".into(),
            latest: "2.1.4".into(),
            changelog: String::new(),
            is_security_release: false,
            download_url: None,
        };
        assert!(info.has_upgrade());
    }

    #[test]
    fn test_version_no_upgrade() {
        use crate::update::checker::VersionInfo;
        let info = VersionInfo {
            current: "2.1.4".into(),
            latest: "2.1.4".into(),
            changelog: String::new(),
            is_security_release: false,
            download_url: None,
        };
        assert!(!info.has_upgrade());
    }

    #[test]
    fn test_version_major_upgrade() {
        use crate::update::checker::VersionInfo;
        let info = VersionInfo {
            current: "1.0.0".into(),
            latest: "2.0.0".into(),
            changelog: String::new(),
            is_security_release: true,
            download_url: None,
        };
        assert!(info.has_upgrade());
        assert!(info.is_security_release);
    }

    #[test]
    fn test_version_with_v_prefix() {
        use crate::update::checker::VersionInfo;
        let info = VersionInfo {
            current: "v2.1.3".into(),
            latest: "v2.1.4".into(),
            changelog: String::new(),
            is_security_release: false,
            download_url: None,
        };
        assert!(info.has_upgrade());
    }

    #[test]
    fn test_version_invalid_format() {
        use crate::update::checker::VersionInfo;
        let info = VersionInfo {
            current: "not-a-version".into(),
            latest: "also-not".into(),
            changelog: String::new(),
            is_security_release: false,
            download_url: None,
        };
        assert!(!info.has_upgrade()); // Invalid versions should not trigger upgrade
    }

    // ===== SandboxType =====

    #[test]
    fn test_sandbox_type_from_os() {
        let st = crate::sandbox::SandboxType::from_os();
        assert!(matches!(
            st,
            crate::sandbox::SandboxType::LimaAlpine
                | crate::sandbox::SandboxType::PodmanAlpine
                | crate::sandbox::SandboxType::Wsl2Alpine
        ));
    }

    #[test]
    fn test_sandbox_type_serialization() {
        let lima = crate::sandbox::SandboxType::LimaAlpine;
        let json = serde_json::to_string(&lima).expect("Serialize failed");
        assert_eq!(json, "\"lima-alpine\"");

        let parsed: crate::sandbox::SandboxType =
            serde_json::from_str(&json).expect("Deserialize failed");
        assert_eq!(parsed, crate::sandbox::SandboxType::LimaAlpine);
    }

    // ===== Install Options =====

    #[test]
    fn test_install_options_defaults() {
        let opts = crate::manager::install::InstallOptions::default();
        assert_eq!(opts.instance_name, "default");
        assert_eq!(opts.claw_version, "latest");
        assert!(!opts.install_browser);
        assert!(!opts.use_native);
        assert_eq!(opts.gateway_port, 3000);
        assert!(opts.api_key.is_none());
    }

    // ===== Monitor Health Enum =====

    #[test]
    fn test_health_serialization() {
        let running = crate::monitor::InstanceHealth::Running;
        let json = serde_json::to_string(&running).expect("Serialize failed");
        assert_eq!(json, "\"running\"");

        let event = crate::monitor::HealthEvent {
            instance_name: "test".into(),
            health: crate::monitor::InstanceHealth::Stopped,
        };
        let json = serde_json::to_string(&event).expect("Serialize failed");
        assert!(json.contains("\"stopped\""));
        assert!(json.contains("\"test\""));
    }

    // ===== Config Manager =====

    #[test]
    fn test_config_path_exists() {
        let path = crate::config::ConfigManager::config_path();
        assert!(path.is_ok());
        let path = path.unwrap();
        assert!(path.to_string_lossy().contains(".clawenv"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }
}
