//! L2 Flow tests: verify install/upgrade/instance command sequences
//! using MockBackend. Parameterized across all claw types.

#![cfg(test)]

use crate::claw::ClawRegistry;
use crate::sandbox::SandboxBackend;
use crate::sandbox::mock::MockBackend;

/// For each builtin claw, verify the install flow sends correct commands.
///
/// The install flow (simplified) does:
///   1. `which {binary}` — check if installed
///   2. `npm install -g {package}@{version}` — install via background script
///   3. `{binary} --version` — verify install
///   4. `{binary} config set apiKey ...` — configure (if supported)
///   5. `nohup {binary} {gateway_cmd}` — start gateway
///
/// We verify that the descriptor's formatted commands appear in the exec() call sequence.
#[tokio::test]
async fn install_commands_use_descriptor_for_all_claws() {
    let registry = ClawRegistry::load();

    for desc in registry.list_all() {
        let mut backend = MockBackend::new(&format!("test-{}", desc.id));

        // Program responses
        backend
            .on_exec(&format!("which {}", desc.cli_binary), "")  // not installed
            .on_exec("clawenv-npm.sh", "")  // write script
            .on_exec("nohup sh", "")        // launch background
            .on_exec("clawenv-npm.done", "0")  // done marker
            .on_exec(&desc.cli_binary, &format!("{} 1.0.0", desc.display_name))  // version
            .set_default_response("");

        // Simulate the key install commands
        let version = "latest";

        // Step 1: check installed
        let which_cmd = format!("which {} 2>/dev/null", desc.cli_binary);
        let _ = backend.exec(&which_cmd).await;

        // Step 2: install script (the real flow writes a script containing the install command)
        let install_cmd = desc.sandbox_install_cmd(version);
        let script = format!(
            "cat > /tmp/clawenv-npm.sh << 'SCRIPTEOF'\n#!/bin/sh\nsudo {} > /tmp/clawenv-npm.log 2>&1\necho $? > /tmp/clawenv-npm.done\nSCRIPTEOF",
            install_cmd
        );
        let _ = backend.exec(&script).await;

        // Step 3: verify version
        let ver_cmd = format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd());
        let ver_output = backend.exec(&ver_cmd).await.unwrap();
        assert!(
            !ver_output.is_empty(),
            "[{}] version check returned empty", desc.id
        );

        // Step 4: configure API key (if supported)
        if let Some(apikey_cmd) = desc.set_apikey_cmd("sk-test") {
            let _ = backend.exec(&format!("{apikey_cmd} 2>/dev/null || true")).await;
        }

        // Step 5: start gateway (if this claw has one)
        if let Some(gateway_cmd) = desc.gateway_start_cmd(desc.default_port) {
            let _ = backend.exec(&format!("nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &")).await;
        }

        // ---- Assertions ----

        // Must have called with the correct binary name, NOT "openclaw" (unless it IS openclaw)
        backend.assert_called_with(&format!("which {}", desc.cli_binary));
        // Must have called with the correct package name
        let expected_pkg = match desc.package_manager {
            crate::claw::descriptor::PackageManager::Npm => &desc.npm_package,
            crate::claw::descriptor::PackageManager::Pip => &desc.pip_package,
        };
        backend.assert_called_with(expected_pkg);
        backend.assert_called_with(&desc.cli_binary);

        // Must NOT contain hardcoded "openclaw" for non-openclaw types
        if desc.id != "openclaw" {
            backend.assert_not_called_with("openclaw");
        }

        // Gateway command must contain the correct port (if gateway_cmd uses {port})
        if desc.gateway_cmd.contains("{port}") {
            backend.assert_called_with(&desc.default_port.to_string());
        }
    }
}

/// Verify upgrade flow uses descriptor commands for all claw types.
#[tokio::test]
async fn upgrade_commands_use_descriptor_for_all_claws() {
    let registry = ClawRegistry::load();

    for desc in registry.list_all() {
        let mut backend = MockBackend::new(&format!("upgrade-{}", desc.id));
        backend.set_default_response("");

        let version = "2.0.0";

        // Step 1: kill old gateway
        let kill_cmd = crate::platform::process::kill_by_name_cmd(&desc.process_names()[0]);
        let _ = backend.exec(&kill_cmd).await;

        // Step 2: upgrade script
        let install_cmd = desc.sandbox_install_cmd(version);
        let script = format!(
            "cat > /tmp/clawenv-upgrade.sh << 'UPGEOF'\n#!/bin/sh\nsudo {} > /tmp/clawenv-upgrade.log 2>&1\necho $? > /tmp/clawenv-upgrade.done\nUPGEOF",
            install_cmd
        );
        let _ = backend.exec(&script).await;

        // Step 3: verify
        let ver_cmd = format!("{} 2>/dev/null || echo unknown", desc.version_check_cmd());
        let _ = backend.exec(&ver_cmd).await;

        // Step 4: restart gateway (if supported)
        if let Some(gateway_cmd) = desc.gateway_start_cmd(desc.default_port) {
            let _ = backend.exec(&format!("nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &")).await;
        }

        // Assertions
        let expected_pkg = match desc.package_manager {
            crate::claw::descriptor::PackageManager::Npm => &desc.npm_package,
            crate::claw::descriptor::PackageManager::Pip => &desc.pip_package,
        };
        backend.assert_called_with(expected_pkg);
        backend.assert_called_with(&desc.cli_binary);

        if desc.id != "openclaw" {
            backend.assert_not_called_with("openclaw");
        }
    }
}

/// Verify instance start/stop uses descriptor for all claw types.
#[tokio::test]
async fn instance_lifecycle_uses_descriptor_for_all_claws() {
    let registry = ClawRegistry::load();

    for desc in registry.list_all() {
        let mut backend = MockBackend::new(&format!("lifecycle-{}", desc.id));
        backend.set_default_response("");

        // Start: kill stale + start gateway
        let kill_cmd = crate::platform::process::kill_by_name_cmd(&desc.process_names()[0]);
        let _ = backend.exec(&kill_cmd).await;

        if let Some(gateway_cmd) = desc.gateway_start_cmd(desc.default_port) {
            let _ = backend.exec(&format!("nohup {gateway_cmd} > /tmp/clawenv-gateway.log 2>&1 &")).await;
        }

        // Stop: kill gateway
        let _ = backend.exec(&kill_cmd).await;

        // Assertions
        backend.assert_called_with(&desc.cli_binary);
        let kill_calls = backend.calls_matching(&kill_cmd);
        assert_eq!(kill_calls.len(), 2, "[{}] expected 2 kill calls, got {}", desc.id, kill_calls.len());

        if desc.id != "openclaw" {
            backend.assert_not_called_with("openclaw");
        }
    }
}

/// Verify that pip-based claws never generate npm commands in upgrade flow.
#[tokio::test]
async fn pip_claws_upgrade_never_uses_npm() {
    let registry = ClawRegistry::load();

    for desc in registry.list_all() {
        if desc.package_manager != crate::claw::descriptor::PackageManager::Pip {
            continue;
        }

        let mut backend = MockBackend::new(&format!("pip-upgrade-{}", desc.id));
        backend.set_default_response("");

        let version = "2.0.0";
        let install_cmd = desc.sandbox_install_cmd(version);
        let _ = backend.exec(&install_cmd).await;

        // pip claws must use pip, never npm
        assert!(install_cmd.contains("pip install"), "[{}] upgrade should use pip, got: {}", desc.id, install_cmd);
        assert!(!install_cmd.contains("npm"), "[{}] upgrade must NOT contain npm, got: {}", desc.id, install_cmd);
        assert!(install_cmd.contains(&desc.pip_package), "[{}] upgrade must contain pip_package, got: {}", desc.id, install_cmd);

        // npm_install_verbose_cmd should still produce npm (but should NOT be called for pip claws)
        let npm_cmd = desc.npm_install_verbose_cmd(version);
        assert!(npm_cmd.contains("npm"), "npm_install_verbose_cmd must always produce npm");
    }
}

/// Verify that npm-based claws never generate pip commands in upgrade flow.
#[tokio::test]
async fn npm_claws_upgrade_never_uses_pip() {
    let registry = ClawRegistry::load();

    for desc in registry.list_all() {
        if desc.package_manager != crate::claw::descriptor::PackageManager::Npm {
            continue;
        }

        let install_cmd = desc.sandbox_install_cmd("latest");
        assert!(install_cmd.contains("npm"), "[{}] upgrade should use npm, got: {}", desc.id, install_cmd);
        assert!(!install_cmd.contains("pip"), "[{}] upgrade must NOT contain pip, got: {}", desc.id, install_cmd);
    }
}

/// Verify that all claw types produce unique, non-empty install commands.
#[tokio::test]
async fn all_claws_produce_distinct_install_commands() {
    let registry = ClawRegistry::load();
    let mut seen = std::collections::HashSet::new();

    for desc in registry.list_all() {
        let cmd = desc.sandbox_install_cmd("latest");
        assert!(!cmd.is_empty(), "[{}] empty sandbox install cmd", desc.id);
        assert!(
            seen.insert(cmd.clone()),
            "[{}] duplicate sandbox install cmd: {}", desc.id, cmd
        );
    }
}
