#[cfg(test)]
mod descriptor_tests {
    use crate::claw::descriptor::*;

    // ---- OpenClaw (default) ----

    #[test]
    fn openclaw_gateway_cmd() {
        let d = openclaw_descriptor();
        assert_eq!(d.gateway_start_cmd(3000).unwrap(), "openclaw gateway --port 3000 --allow-unconfigured");
        assert_eq!(d.gateway_start_cmd(8080).unwrap(), "openclaw gateway --port 8080 --allow-unconfigured");
    }

    #[test]
    fn openclaw_version_cmd() {
        let d = openclaw_descriptor();
        assert_eq!(d.version_check_cmd(), "openclaw --version");
    }

    #[test]
    fn openclaw_npm_install() {
        let d = openclaw_descriptor();
        assert_eq!(d.npm_install_cmd("latest"), "npm install -g openclaw@latest");
        assert_eq!(d.npm_install_cmd("1.2.3"), "npm install -g openclaw@1.2.3");
    }

    #[test]
    fn openclaw_npm_install_verbose() {
        let d = openclaw_descriptor();
        assert_eq!(
            d.npm_install_verbose_cmd("latest"),
            "npm install -g --loglevel verbose openclaw@latest"
        );
    }

    #[test]
    fn openclaw_sandbox_install() {
        let d = openclaw_descriptor();
        assert_eq!(d.sandbox_install_cmd("latest"), "npm install -g --loglevel verbose openclaw@latest");
        assert_eq!(d.sandbox_install_cmd("1.2.3"), "npm install -g --loglevel verbose openclaw@1.2.3");
    }

    #[test]
    fn openclaw_apikey_cmd() {
        let d = openclaw_descriptor();
        let cmd = d.set_apikey_cmd("sk-test123");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap(), "openclaw config set apiKey 'sk-test123'");
    }

    #[test]
    fn openclaw_mcp_cmd() {
        let d = openclaw_descriptor();
        let cmd = d.mcp_register_cmd("clawenv", r#"{"command":"node"}"#);
        assert!(cmd.is_some());
        assert!(cmd.unwrap().contains("openclaw mcp set clawenv"));
    }

    #[test]
    fn openclaw_process_names() {
        let d = openclaw_descriptor();
        let names = d.process_names();
        assert!(names.contains(&"openclaw gateway".to_string()));
        assert!(names.contains(&"openclaw-gateway".to_string()));
    }

    #[test]
    fn openclaw_supports_features() {
        let d = openclaw_descriptor();
        assert!(d.supports_mcp);
        assert!(d.supports_browser);
        assert!(d.has_gateway_ui);
        assert!(d.supports_native);
        assert!(!d.uses_python_mcp());
    }

    // ---- Hermes Agent ----

    #[test]
    fn hermes_sandbox_install() {
        let registry = crate::claw::ClawRegistry::load();
        let d = registry.get("hermes");
        assert_eq!(d.package_manager, PackageManager::GitPip);
        let cmd = d.sandbox_install_cmd("latest");
        assert!(cmd.contains("git clone"), "should git clone: {cmd}");
        assert!(cmd.contains("NousResearch/hermes-agent"), "should clone hermes repo: {cmd}");
        assert!(cmd.contains("uv venv"), "should create venv: {cmd}");
        assert!(cmd.contains("uv pip install"), "should uv pip install: {cmd}");
        assert!(cmd.contains("[termux,messaging,web]"), "should install with musl-safe extras: {cmd}");
        assert!(cmd.contains("ln -sf"), "should symlink binary: {cmd}");
        assert!(cmd.contains("/usr/local/bin/hermes"), "should symlink to /usr/local/bin: {cmd}");
        // Specific version → branch tag
        let cmd_ver = d.sandbox_install_cmd("0.3.0");
        assert!(cmd_ver.contains("v0.3.0"), "should checkout version tag: {cmd_ver}");
    }

    #[test]
    fn hermes_has_gateway() {
        let registry = crate::claw::ClawRegistry::load();
        let d = registry.get("hermes");
        assert!(d.gateway_start_cmd(3000).is_some(), "hermes should have gateway (API Server)");
        assert!(d.has_gateway_ui);
    }

    #[test]
    fn hermes_no_native() {
        let registry = crate::claw::ClawRegistry::load();
        let d = registry.get("hermes");
        assert!(!d.supports_native);
    }

    #[test]
    fn hermes_uses_python_mcp() {
        let registry = crate::claw::ClawRegistry::load();
        let d = registry.get("hermes");
        assert!(d.uses_python_mcp());
        assert!(d.supports_mcp);
    }

    #[test]
    fn hermes_has_sandbox_provision() {
        let registry = crate::claw::ClawRegistry::load();
        let d = registry.get("hermes");
        assert!(!d.sandbox_provision.is_empty(), "hermes needs sandbox_provision for Python");
        assert!(d.sandbox_provision.contains(&"py3-pip".to_string()));
    }

    #[test]
    fn hermes_mcp_register() {
        let registry = crate::claw::ClawRegistry::load();
        let d = registry.get("hermes");
        let cmd = d.mcp_register_cmd("clawenv", r#"{"command":"python3"}"#);
        assert!(cmd.is_some());
        assert!(cmd.unwrap().contains("hermes mcp add clawenv"));
    }

    // ---- Registry-loaded descriptors: command formatting for all claw types ----

    /// Helper: load registry and test a specific claw's commands.
    fn assert_claw_commands(
        id: &str,
        expected_binary: &str,
        expected_port: u16,
        has_apikey: bool,
        has_mcp: bool,
    ) {
        let registry = crate::claw::ClawRegistry::load();
        let d = registry.get(id);

        // Basic identity
        assert_eq!(d.id, id, "id mismatch for {id}");
        assert!(!d.display_name.is_empty(), "display_name empty for {id}");
        assert!(!d.logo.is_empty(), "logo empty for {id}");
        assert_eq!(d.default_port, expected_port, "default_port mismatch for {id}");

        // CLI binary
        assert_eq!(d.cli_binary, expected_binary, "cli_binary mismatch for {id}");

        // version_check_cmd must contain the binary
        let ver_cmd = d.version_check_cmd();
        assert!(ver_cmd.starts_with(expected_binary), "version_check_cmd doesn't start with binary for {id}: {ver_cmd}");

        // gateway_start_cmd: if gateway_cmd is non-empty, must contain port and binary
        if let Some(gw) = d.gateway_start_cmd(expected_port) {
            assert!(gw.contains(expected_binary), "gateway cmd missing binary for {id}: {gw}");
            assert!(gw.contains(&expected_port.to_string()), "gateway cmd missing port for {id}: {gw}");
        }

        // sandbox_install_cmd must contain the package
        let install = d.sandbox_install_cmd("latest");
        match d.package_manager {
            PackageManager::Npm => {
                assert!(install.contains(&d.npm_package), "sandbox install missing npm package for {id}: {install}");
            }
            PackageManager::Pip => {
                assert!(install.contains(&d.pip_package), "sandbox install missing pip package for {id}: {install}");
            }
            PackageManager::GitPip => {
                assert!(install.contains("git clone"), "sandbox install missing git clone for {id}: {install}");
                assert!(install.contains(&d.git_repo), "sandbox install missing git_repo for {id}: {install}");
            }
        }

        // process_names must contain binary
        let pns = d.process_names();
        assert!(pns.iter().any(|p| p.contains(expected_binary)), "process_names missing binary for {id}: {pns:?}");

        // API key support
        if has_apikey {
            assert!(d.set_apikey_cmd("test").is_some(), "expected apikey support for {id}");
        }

        // MCP support
        if has_mcp {
            assert!(d.mcp_register_cmd("x", "{}").is_some(), "expected mcp support for {id}");
        } else {
            assert!(d.mcp_register_cmd("x", "{}").is_none(), "unexpected mcp support for {id}");
        }
    }

    #[test]
    fn all_builtin_claws_have_valid_commands() {
        //       id,          binary,       port, apikey, mcp
        assert_claw_commands("openclaw",   "openclaw",   3000, true,  true);
        assert_claw_commands("hermes",     "hermes",     3000, true,  true);
    }

    // ---- Edge cases ----

    #[test]
    fn empty_apikey_cmd_returns_none() {
        let mut d = openclaw_descriptor();
        d.config_apikey_cmd = String::new();
        assert!(d.set_apikey_cmd("key").is_none());
    }

    #[test]
    fn empty_mcp_cmd_returns_none() {
        let mut d = openclaw_descriptor();
        d.mcp_set_cmd = String::new();
        assert!(d.mcp_register_cmd("name", "json").is_none());
    }

    #[test]
    fn empty_gateway_cmd_returns_none() {
        let mut d = openclaw_descriptor();
        d.gateway_cmd = String::new();
        assert!(d.gateway_start_cmd(3000).is_none());
    }

    #[test]
    fn apikey_with_special_chars() {
        let d = openclaw_descriptor();
        let cmd = d.set_apikey_cmd("sk-abc'def\"ghi").unwrap();
        assert!(cmd.contains("sk-abc'def\"ghi"));
    }

    #[test]
    fn gateway_cmd_port_substitution() {
        let mut d = openclaw_descriptor();
        d.cli_binary = "testclaw".into();
        d.gateway_cmd = "serve --port {port} --host 0.0.0.0".into();
        assert_eq!(d.gateway_start_cmd(9999).unwrap(), "testclaw serve --port 9999 --host 0.0.0.0");
    }
}

#[cfg(test)]
mod registry_tests {
    use crate::claw::ClawRegistry;

    #[test]
    fn registry_loads_all_builtin_claws() {
        let reg = ClawRegistry::load();
        let ids = reg.list_ids();
        assert!(ids.contains(&"openclaw"), "missing openclaw");
        assert!(ids.contains(&"hermes"), "missing hermes");
        assert!(ids.len() >= 2, "expected at least 2 builtin claws, got {}", ids.len());
    }

    #[test]
    fn registry_list_all_puts_openclaw_first() {
        let reg = ClawRegistry::load();
        let all = reg.list_all();
        assert!(!all.is_empty(), "registry should not be empty");
        assert_eq!(all[0].id, "openclaw", "openclaw must be listed first");
    }

    #[test]
    fn registry_get_returns_correct_descriptor() {
        let reg = ClawRegistry::load();
        let d = reg.get("hermes");
        assert_eq!(d.id, "hermes");
        assert_eq!(d.display_name, "Hermes Agent");
    }

    #[test]
    fn registry_get_unknown_falls_back_to_openclaw() {
        let reg = ClawRegistry::load();
        let d = reg.get("nonexistent-claw-xyz");
        assert_eq!(d.id, "openclaw");
    }

    #[test]
    fn registry_get_strict_returns_error_for_unknown() {
        let reg = ClawRegistry::load();
        let result = reg.get_strict("nonexistent-claw-xyz");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown claw type"), "unexpected error: {err}");
    }

    #[test]
    fn registry_list_all_matches_list_ids() {
        let reg = ClawRegistry::load();
        let ids = reg.list_ids();
        let all = reg.list_all();
        assert_eq!(ids.len(), all.len());
    }

    #[test]
    fn registry_custom_registration() {
        let mut reg = ClawRegistry::load();
        let before = reg.list_ids().len();
        reg.register(crate::claw::ClawDescriptor {
            id: "test-custom".into(),
            display_name: "Test Custom Claw".into(),
            logo: "🧪".into(),
            package_manager: crate::claw::descriptor::PackageManager::Npm,
            npm_package: "test-custom-claw".into(),
            pip_package: String::new(),
            git_repo: String::new(),
            pip_extras: String::new(),
            sandbox_provision: vec![],
            cli_binary: "testclaw".into(),
            gateway_cmd: "serve --port {port}".into(),
            version_cmd: "--version".into(),
            config_apikey_cmd: String::new(),
            mcp_set_cmd: String::new(),
            default_port: 4000,
            supports_mcp: false,
            supports_browser: false,
            has_gateway_ui: true,
            supports_native: true,
            mcp_runtime: "node".into(),
        });
        assert_eq!(reg.list_ids().len(), before + 1);
        assert_eq!(reg.get("test-custom").display_name, "Test Custom Claw");
    }

    #[test]
    fn registry_openclaw_always_present_even_if_toml_missing() {
        // ClawRegistry::load() ensures openclaw is always there via or_insert
        let reg = ClawRegistry::load();
        let d = reg.get("openclaw");
        assert_eq!(d.cli_binary, "openclaw");
        assert!(d.supports_mcp);
    }
}
