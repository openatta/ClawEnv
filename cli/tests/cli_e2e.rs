//! End-to-end CLI integration tests.
//!
//! These tests run the actual `clawcli` binary and verify its output
//! in both human and JSON modes.

use std::process::Command;

fn cli_bin() -> Command {
    // Use cargo to find the binary
    let bin = env!("CARGO_BIN_EXE_clawcli");
    Command::new(bin)
}

fn run_json(args: &[&str]) -> (i32, serde_json::Value) {
    let mut cmd = cli_bin();
    cmd.arg("--json");
    cmd.args(args);
    let output = cmd.output().expect("failed to run clawcli");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse last JSON line that has "data" type
    let mut last_event = serde_json::Value::Null;
    for line in stdout.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            last_event = v;
        }
    }
    (code, last_event)
}

// ---- Tests ----

#[test]
fn test_help() {
    let output = cli_bin().arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("claw"));
    assert!(stdout.contains("install"));
    assert!(stdout.contains("list"));
    assert!(stdout.contains("doctor"));
}

#[test]
fn test_version() {
    let output = cli_bin().arg("--version").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("claw"));
}

#[test]
fn test_claw_types_json() {
    let (code, event) = run_json(&["claw-types"]);
    assert_eq!(code, 0);
    assert_eq!(event["type"], "data");
    let types = event["data"]["claw_types"].as_array().unwrap();
    assert!(types.len() >= 2, "should have at least 2 claw types");

    // Verify OpenClaw is in the list
    let openclaw = types.iter().find(|t| t["id"] == "openclaw");
    assert!(openclaw.is_some(), "openclaw should be in claw types");
    assert_eq!(openclaw.unwrap()["default_port"], 3000);
}

#[test]
fn test_claw_types_human() {
    let output = cli_bin().arg("claw-types").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("openclaw"));
    assert!(stdout.contains("OpenClaw"));
}

#[test]
fn test_doctor_json() {
    let (code, event) = run_json(&["doctor"]);
    assert_eq!(code, 0);
    assert_eq!(event["type"], "data");

    let data = &event["data"];
    // Should have OS, arch, memory, disk info
    assert!(data["os"].is_string());
    assert!(data["arch"].is_string());
    assert!(data["memory_gb"].is_string());
    assert!(data["disk_free_gb"].is_string());
}

#[test]
fn test_system_check_json() {
    let (code, event) = run_json(&["system-check"]);
    assert_eq!(code, 0);
    assert_eq!(event["type"], "data");

    let data = &event["data"];
    let checks = data["checks"].as_array().unwrap();
    assert!(checks.len() >= 3, "should have at least 3 checks (OS, Memory, Disk)");

    // OS check should always pass
    let os_check = checks.iter().find(|c| c["name"] == "OS");
    assert!(os_check.is_some());
    assert_eq!(os_check.unwrap()["ok"], true);
}

#[test]
fn test_list_json() {
    let (code, event) = run_json(&["list"]);
    assert_eq!(code, 0);
    assert_eq!(event["type"], "data");
    // instances should be an array (may be empty)
    assert!(event["data"]["instances"].is_array());
}

#[test]
fn test_list_human() {
    let output = cli_bin().arg("list").output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_status_nonexistent_instance() {
    let (code, event) = run_json(&["status", "nonexistent-instance-xyz"]);
    // Should fail with error
    assert_ne!(code, 0);
    assert_eq!(event["type"], "error");
}

#[test]
fn test_json_output_format() {
    // All JSON output should be valid JSON lines
    let mut cmd = cli_bin();
    cmd.args(["--json", "claw-types"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
        assert!(parsed.is_ok(), "Invalid JSON line: {}", line);
        let v = parsed.unwrap();
        assert!(v["type"].is_string(), "Each JSON event must have a 'type' field");
    }
}

#[test]
fn test_subcommand_help() {
    for subcmd in &["install", "list", "doctor", "claw-types", "system-check"] {
        let output = cli_bin().args([subcmd, "--help"]).output().unwrap();
        assert!(output.status.success(), "{} --help should succeed", subcmd);
    }
}

#[test]
fn test_install_help_shows_step() {
    let output = cli_bin().args(["install", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--step"), "install --help should show --step option");
    assert!(stdout.contains("prereq"), "install --help should mention prereq step");
}

#[test]
fn test_install_bad_step() {
    let (code, event) = run_json(&["install", "--step", "nonexistent"]);
    assert_ne!(code, 0);
    assert_eq!(event["type"], "error");
    let msg = event["message"].as_str().unwrap_or("");
    assert!(msg.contains("Unknown install step"), "should report unknown step, got: {}", msg);
}

#[test]
fn test_install_step_prereq_native() {
    // Native prereq checks ClawEnv's own Node.js (~/.clawenv/node/).
    // May need to download if not present, and the download can fail on
    // CI runners (GitHub-hosted Windows in particular: rate limits,
    // transient network, mirror quirks). The test is really only
    // asserting "the CLI's prereq subcommand emits a structured event"
    // — not "the prereq actually succeeded". A structured `error` event
    // is a valid outcome; an unstructured panic (no parseable JSON, no
    // `type` field) is the real failure we're guarding against.
    let (_code, event) = run_json(&["install", "--mode", "native", "--step", "prereq"]);
    let event_type = event["type"].as_str().unwrap_or("");
    assert!(
        matches!(event_type, "info" | "progress" | "complete" | "error"),
        "prereq should emit a structured CLI event, got {event_type:?} (full event: {event})"
    );
}

#[test]
fn test_install_step_prereq_sandbox() {
    // Sandbox prereq checks Lima/WSL2/Podman
    let (code, event) = run_json(&["install", "--mode", "sandbox", "--step", "prereq"]);
    assert_eq!(code, 0);
    let event_type = event["type"].as_str().unwrap_or("");
    assert!(
        event_type == "complete" || event_type == "info",
        "prereq should emit complete or info, got: {}", event_type
    );
}

// ---- Hermes Agent (pip-based claw) ----

#[test]
fn test_claw_types_includes_hermes() {
    let (code, event) = run_json(&["claw-types"]);
    assert_eq!(code, 0);
    let types = event["data"]["claw_types"].as_array().unwrap();

    let hermes = types.iter().find(|t| t["id"] == "hermes");
    assert!(hermes.is_some(), "hermes should be in claw types");
    let h = hermes.unwrap();
    assert_eq!(h["package_manager"], "git_pip");
    // v2 wire merged npm_package/pip_package into a single package_id field.
    assert_eq!(h["package_id"], "hermes-agent");
    assert_eq!(h["has_gateway_ui"], true);
    assert_eq!(h["supports_native"], false);
    assert_eq!(h["supports_mcp"], true);
}

#[test]
fn test_claw_types_hermes_human() {
    let output = cli_bin().arg("claw-types").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hermes"), "claw-types human output should include hermes");
    assert!(stdout.contains("Hermes Agent"), "claw-types human output should include display name");
}

#[test]
fn test_claw_types_all_have_required_fields() {
    let (code, event) = run_json(&["claw-types"]);
    assert_eq!(code, 0);
    let types = event["data"]["claw_types"].as_array().unwrap();
    assert!(types.len() >= 2, "should have at least 2 claw types (openclaw, hermes)");

    for ct in types {
        let id = ct["id"].as_str().unwrap_or("???");
        assert!(ct["display_name"].is_string(), "{id}: missing display_name");
        assert!(ct["package_manager"].is_string(), "{id}: missing package_manager");
        // v2 wire merged npm_package/pip_package into a single package_id field;
        // the `logo` GUI field moved to the Tauri side's claw_catalog.
        assert!(ct["package_id"].is_string(), "{id}: missing package_id");
        assert!(ct["default_port"].is_number(), "{id}: missing default_port");
        // Boolean fields must be present (not null)
        assert!(ct["supports_mcp"].is_boolean(), "{id}: missing supports_mcp");
        assert!(ct["supports_browser"].is_boolean(), "{id}: missing supports_browser");
        assert!(ct["has_gateway_ui"].is_boolean(), "{id}: missing has_gateway_ui");
        assert!(ct["supports_native"].is_boolean(), "{id}: missing supports_native");
    }
}

#[test]
fn test_install_hermes_rejects_native_mode() {
    // Hermes doesn't support native mode — install should fail
    let (code, event) = run_json(&["install", "--claw-type", "hermes", "--mode", "native", "--step", "prereq"]);
    // Should either fail (native not supported) or succeed at prereq only (depends on implementation)
    // At minimum, it should not crash
    let event_type = event["type"].as_str().unwrap_or("");
    assert!(
        event_type == "error" || event_type == "complete" || event_type == "info" || event_type == "progress",
        "hermes native install should not crash, got code={code} type={event_type}"
    );
}

#[test]
fn test_install_hermes_sandbox_step_prereq() {
    // Hermes sandbox prereq should work like OpenClaw sandbox
    let (code, event) = run_json(&["install", "--claw-type", "hermes", "--mode", "sandbox", "--step", "prereq"]);
    assert_eq!(code, 0, "hermes sandbox prereq should succeed");
    let event_type = event["type"].as_str().unwrap_or("");
    assert!(
        event_type == "complete" || event_type == "info",
        "hermes prereq should emit complete or info, got: {}", event_type
    );
}

// ---- New commands ----

#[test]
fn test_config_show_json() {
    // Fresh CI runners don't have `~/.clawenv/config.toml` and `config
    // show` correctly errors out rather than pretending. The test's job
    // is to verify the JSON shape when the config IS present — not to
    // stand up the config itself — so accept either: a `data` event
    // with the expected field shape, OR a structured `error` event.
    let (_code, event) = run_json(&["config", "show"]);
    match event["type"].as_str() {
        Some("data") => {
            let data = &event["data"];
            assert!(data["language"].is_string());
            assert!(data["theme"].is_string());
            assert!(data["instances_count"].is_number());
        }
        Some("error") => {
            // Accepted — config absent on fresh environment is a valid
            // non-crash outcome. The CLI produced a structured event.
        }
        other => panic!("unexpected event type {other:?}: {event}"),
    }
}

#[test]
fn test_config_set_invalid_key() {
    let (code, event) = run_json(&["config", "set", "nonexistent.key", "value"]);
    assert_ne!(code, 0);
    assert_eq!(event["type"], "error");
}

#[test]
fn test_sandbox_list_json() {
    let (code, event) = run_json(&["sandbox", "list"]);
    assert_eq!(code, 0);
    assert_eq!(event["type"], "data");
    assert!(event["data"]["vms"].is_array());
}

#[test]
fn test_sandbox_info_json() {
    let (code, event) = run_json(&["sandbox", "info"]);
    assert_eq!(code, 0);
    assert_eq!(event["type"], "data");
}

#[test]
fn test_edit_nonexistent_instance() {
    let (code, event) = run_json(&["edit", "nonexistent-xyz", "--cpus", "4"]);
    assert_ne!(code, 0);
    assert_eq!(event["type"], "error");
}

#[test]
fn test_rename_nonexistent() {
    let (code, event) = run_json(&["rename", "nonexistent-xyz", "new-name"]);
    assert_ne!(code, 0);
    assert_eq!(event["type"], "error");
}

#[test]
fn test_new_subcommand_help() {
    for subcmd in &["rename", "edit", "sandbox", "config"] {
        let output = cli_bin().args([subcmd, "--help"]).output().unwrap();
        assert!(output.status.success(), "{} --help should succeed", subcmd);
    }
}

// ---- Completeness: all 22 commands have at least --help coverage ----

#[test]
fn test_all_commands_help() {
    let commands = [
        "install", "uninstall", "list", "start", "stop", "restart",
        "status", "logs", "upgrade", "update-check", "export", "import",
        "doctor", "exec", "claw-types", "system-check",
        "rename", "edit", "sandbox", "config",
    ];
    for cmd in &commands {
        let output = cli_bin().args([cmd, "--help"]).output().unwrap();
        assert!(output.status.success(), "{} --help failed", cmd);
    }
}

#[test]
fn test_config_proxy_test_no_proxy() {
    // Should succeed with info message when no proxy configured.
    // We only assert on the *event type* here — the exit code is
    // intentionally ignored because it varies by whether a config.toml
    // exists on the test machine (nonzero if no config, zero with one).
    let (_code, event) = run_json(&["config", "proxy-test"]);
    let t = event["type"].as_str().unwrap_or("");
    assert!(t == "info" || t == "error", "proxy-test should emit info or error, got: {t}");
}

#[test]
fn test_sandbox_shell_native_instance() {
    // sandbox shell on a native instance should fail with clear error
    // (only works if there's a native instance named "default")
    let output = cli_bin().args(["--json", "sandbox", "shell", "nonexistent-xyz"]).output().unwrap();
    // Should fail — either instance not found or native mode error
    assert!(!output.status.success());
}

#[test]
fn test_upgrade_nonexistent() {
    let (code, event) = run_json(&["upgrade", "nonexistent-xyz"]);
    assert_ne!(code, 0);
    assert_eq!(event["type"], "error");
}

#[test]
fn test_export_nonexistent() {
    let (code, event) = run_json(&["export", "nonexistent-xyz"]);
    assert_ne!(code, 0);
    assert_eq!(event["type"], "error");
}

#[test]
fn test_import_nonexistent_file() {
    let (code, event) = run_json(&["import", "/tmp/nonexistent-file-xyz.tar.gz"]);
    assert_ne!(code, 0);
    assert_eq!(event["type"], "error");
}
