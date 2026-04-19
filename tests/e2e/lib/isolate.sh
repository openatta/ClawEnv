#!/bin/bash
# Isolate ClawEnv state to a test-scoped $HOME directory.
#
# Why $HOME override: core/src/* uses `dirs::home_dir()` which reads $HOME
# on macOS. Setting HOME=/tmp/clawenv-e2e-<ts> reroutes EVERY clawenv
# path — config.toml, lima VMs, workspaces, cache — into the isolated
# tree. Zero code change needed. Restores after teardown.
#
# Sourced by run.sh before any clawcli invocation.

# Generate a unique test home. Called once per run.
e2e_isolate_setup() {
    export E2E_TEST_HOME="${E2E_TEST_HOME:-/tmp/clawenv-e2e-$(date +%s)-$$}"
    mkdir -p "$E2E_TEST_HOME"
    # Save user's real HOME so we can refer to it for bundle-dir default
    # (~/Desktop/ClawEnv is the user's real desktop, not the isolated one).
    export E2E_REAL_HOME="${HOME}"
    export HOME="$E2E_TEST_HOME"
    echo "[isolate] HOME=$HOME (was $E2E_REAL_HOME)" >&2
}

# Call after all scenarios finish. Optionally keep the dir for post-mortem
# (--keep-home flag in run.sh).
e2e_isolate_teardown() {
    local keep="${1:-0}"
    # Restore real HOME before we rm, just in case (defensive).
    local test_home="$HOME"
    export HOME="$E2E_REAL_HOME"

    if [ "$keep" = "1" ]; then
        echo "[isolate] keeping $test_home for post-mortem" >&2
        return 0
    fi

    # Kill any Lima VMs that might still reference this test home, then
    # rm. `limactl delete --force` is idempotent on missing VMs.
    if [ -d "$test_home/.clawenv/lima" ]; then
        LIMA_HOME="$test_home/.clawenv/lima" \
            "$E2E_REAL_HOME/.clawenv/bin/limactl" list 2>/dev/null | \
            awk 'NR>1 {print $1}' | \
            while read -r vm; do
                LIMA_HOME="$test_home/.clawenv/lima" \
                    "$E2E_REAL_HOME/.clawenv/bin/limactl" delete --force "$vm" 2>/dev/null || true
            done
    fi
    rm -rf "$test_home"
    echo "[isolate] removed $test_home" >&2
}

# Dump test log to ~/Desktop/ClawEnv/logs/ so it survives teardown.
e2e_archive_log() {
    local scenario="$1"
    local status="$2"  # pass|fail
    local logs_dir="$E2E_REAL_HOME/Desktop/ClawEnv/logs"
    mkdir -p "$logs_dir"
    local ts=$(date +%Y%m%d-%H%M%S)
    local dest="$logs_dir/${scenario}-${status}-${ts}.log"
    if [ -f "$E2E_TEST_HOME/clawenv-e2e.log" ]; then
        cp "$E2E_TEST_HOME/clawenv-e2e.log" "$dest"
        echo "[isolate] log archived → $dest" >&2
    fi
}
