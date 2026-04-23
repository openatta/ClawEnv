//! Smoke tests for the `clawops` binary.
//!
//! Verifies --help across all subcommands and core read-only commands
//! (claw list, download list, native status, native doctor, download doctor).
//! Does NOT exercise start/stop/fetch which require real network/VMs.

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_prints_five_subcommands() {
    let out = Command::cargo_bin("clawops").unwrap()
        .arg("--help")
        .assert()
        .success();
    let output = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    for cmd in ["claw", "sandbox", "native", "download", "instance"] {
        assert!(output.contains(cmd), "missing subcommand `{cmd}` in --help:\n{output}");
    }
}

#[test]
fn each_group_has_help() {
    for group in ["claw", "sandbox", "native", "download", "instance"] {
        Command::cargo_bin("clawops").unwrap()
            .args([group, "--help"])
            .assert()
            .success();
    }
}

#[test]
fn claw_list_returns_both_products() {
    Command::cargo_bin("clawops").unwrap()
        .args(["claw", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hermes"))
        .stdout(predicate::str::contains("openclaw"));
}

#[test]
fn claw_list_json_is_valid_json() {
    let out = Command::cargo_bin("clawops").unwrap()
        .args(["--json", "claw", "list"])
        .assert()
        .success();
    let s = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&s).expect("json output");
    assert!(v.is_array());
}

#[test]
fn claw_update_preview_includes_json_flag_when_requested() {
    let out = Command::cargo_bin("clawops").unwrap()
        .args(["--json", "claw", "update", "openclaw", "--json", "--yes"])
        .assert()
        .success();
    let s = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let args = v["args"].as_array().unwrap();
    let has_json = args.iter().any(|x| x == "--json");
    let has_yes = args.iter().any(|x| x == "--yes");
    assert!(has_json && has_yes, "args: {args:?}");
}

#[test]
fn claw_update_unknown_fails() {
    Command::cargo_bin("clawops").unwrap()
        .args(["claw", "update", "nonexistent"])
        .assert()
        .failure();
}

#[test]
fn download_list_shows_catalog_entries() {
    let out = Command::cargo_bin("clawops").unwrap()
        .args(["--json", "download", "list"])
        .assert()
        .success();
    let s = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert!(v.is_array());
    let arr = v.as_array().unwrap();
    assert!(!arr.is_empty(), "catalog should have at least the built-in entries");
}

#[test]
fn download_list_filtered_by_os() {
    let out = Command::cargo_bin("clawops").unwrap()
        .args(["--json", "download", "list", "--os", "macos"])
        .assert()
        .success();
    let s = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    for item in v.as_array().unwrap() {
        assert_eq!(item["os"], "macos");
    }
}

#[test]
fn download_doctor_returns_report() {
    Command::cargo_bin("clawops").unwrap()
        .args(["--json", "download", "doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("issues"));
}

#[test]
fn native_doctor_returns_report() {
    Command::cargo_bin("clawops").unwrap()
        .args(["--json", "native", "doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("issues"));
}

#[test]
fn native_status_runs() {
    Command::cargo_bin("clawops").unwrap()
        .args(["--json", "native", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("clawenv_home"));
}

#[test]
fn native_components_returns_array() {
    let out = Command::cargo_bin("clawops").unwrap()
        .args(["--json", "native", "components"])
        .assert()
        .success();
    let s = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert!(v.is_array());
    let arr = v.as_array().unwrap();
    let names: Vec<&str> = arr.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"node"));
    assert!(names.contains(&"git"));
}

#[test]
fn native_upgrade_unknown_version_errs_cleanly() {
    // Stage B: upgrade_node is wired for real. We pass a non-existent version
    // so catalog lookup fails fast without touching the network.
    Command::cargo_bin("clawops").unwrap()
        .args(["native", "upgrade", "node", "--to", "0.0.0-nonexistent"])
        .assert()
        .failure();
}

// ——— Stage C1: instance subcommands ———

fn with_isolated_clawenv_home<F: FnOnce(&std::path::Path)>(f: F) {
    let tmp = tempfile::TempDir::new().unwrap();
    f(tmp.path());
}

#[test]
fn instance_list_empty_on_fresh_home() {
    with_isolated_clawenv_home(|home| {
        let out = Command::cargo_bin("clawops").unwrap()
            .env("CLAWENV_HOME", home)
            .args(["--json", "instance", "list"])
            .assert()
            .success();
        let s = String::from_utf8_lossy(&out.get_output().stdout);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert!(v.is_array());
        assert!(v.as_array().unwrap().is_empty());
    });
}

#[test]
fn instance_info_missing_errs() {
    with_isolated_clawenv_home(|home| {
        Command::cargo_bin("clawops").unwrap()
            .env("CLAWENV_HOME", home)
            .args(["instance", "info", "ghost"])
            .assert()
            .failure();
    });
}

#[test]
fn instance_destroy_missing_errs() {
    with_isolated_clawenv_home(|home| {
        Command::cargo_bin("clawops").unwrap()
            .env("CLAWENV_HOME", home)
            .args(["instance", "destroy", "ghost"])
            .assert()
            .failure();
    });
}

#[test]
fn instance_create_unknown_claw_errs() {
    with_isolated_clawenv_home(|home| {
        Command::cargo_bin("clawops").unwrap()
            .env("CLAWENV_HOME", home)
            .args([
                "instance", "create",
                "--name", "test",
                "--claw", "nonexistent-claw",
                "--backend", "lima",
            ])
            .assert()
            .failure();
    });
}

#[test]
fn instance_create_native_for_hermes_errs() {
    // Hermes doesn't support native — creation should fail preflight.
    with_isolated_clawenv_home(|home| {
        Command::cargo_bin("clawops").unwrap()
            .env("CLAWENV_HOME", home)
            .args([
                "instance", "create",
                "--name", "test",
                "--claw", "hermes",
                "--backend", "native",
            ])
            .assert()
            .failure();
    });
}

// ——— Stage C3: claw --execute ———

#[test]
fn claw_status_preview_still_works_without_execute() {
    let out = Command::cargo_bin("clawops").unwrap()
        .args(["--json", "claw", "status", "hermes"])
        .assert()
        .success();
    let s = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    // Non-execute mode emits CommandPreview shape (has `args`, no `exit_code`).
    assert!(v["args"].is_array());
    assert!(v.get("exit_code").is_none());
}

#[test]
fn claw_version_execute_emits_execution_report_shape() {
    // --execute on `hermes version` against native runner: hermes likely not
    // installed, so it'll fail to spawn. The CLI emits an ExecutionReport
    // with exit_code != 0 and exits non-zero.
    let out = Command::cargo_bin("clawops").unwrap()
        .args(["--json", "claw", "version", "hermes", "--execute"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Should emit an ExecutionReport-shaped object (claw + runner + exit_code).
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("expected JSON, got:\n{stdout}\nerr: {e}"));
    assert_eq!(v["claw"], "hermes");
    assert!(v["runner"].is_string());
    assert!(v["exit_code"].is_number());
}

#[test]
fn download_list_includes_stage_b_artifacts() {
    // Stage B populated catalog with real node/git/lima entries.
    let out = Command::cargo_bin("clawops").unwrap()
        .args(["--json", "download", "list"])
        .assert()
        .success();
    let s = String::from_utf8_lossy(&out.get_output().stdout);
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    let names: std::collections::HashSet<&str> = v.as_array().unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    // node + git + lima + alpine-rootfs should all be present.
    for expected in ["node", "git", "lima", "alpine-rootfs"] {
        assert!(names.contains(expected), "missing artifact {expected} in {names:?}");
    }
}
