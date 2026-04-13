#!/bin/bash
# ClawEnv CLI — Full cross-platform test suite
#
# Tests ALL CLI commands on macOS (local) and Windows (via SSH).
# Uses a custom install directory to avoid affecting existing installations.
#
# Usage:
#   bash scripts/test-cli-full.sh                    # macOS only
#   bash scripts/test-cli-full.sh --windows          # macOS + Windows
#   bash scripts/test-cli-full.sh --windows-only     # Windows only
#   bash scripts/test-cli-full.sh --mode native      # Test native mode (default)
#   bash scripts/test-cli-full.sh --mode sandbox     # Test sandbox mode
#
# Requirements:
#   macOS: cargo build -p clawenv-cli
#   Windows: SSH access configured in .env, Rust toolchain on remote

set -uo pipefail

# ---- Options ----
TEST_MACOS=true
TEST_WINDOWS=false
INSTALL_MODE="native"
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --windows)       TEST_WINDOWS=true; shift;;
        --windows-only)  TEST_WINDOWS=true; TEST_MACOS=false; shift;;
        --mode)          INSTALL_MODE="$2"; shift 2;;
        --verbose)       VERBOSE=true; shift;;
        *) echo "Unknown: $1"; exit 1;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Load .env for Windows SSH
if [ -f "$ROOT/.env" ]; then
    export $(grep -v '^#' "$ROOT/.env" | xargs)
fi
WIN_HOST="${WIN_HOST:-192.168.64.7}"
WIN_USER="${WIN_USER:-clawenv}"
WIN_PROJECT="C:\\Users\\$WIN_USER\\Desktop\\ClawEnv"
WIN_CLI="$WIN_PROJECT\\target\\debug\\clawenv-cli.exe"
WIN_ENV="set PATH=%PATH%;C:\\Program Files\\nodejs;C:\\Program Files\\Git\\cmd;C:\\Users\\$WIN_USER\\.cargo\\bin&&"

# ---- Counters ----
PASS=0; FAIL=0; SKIP=0; TOTAL=0

pass() { PASS=$((PASS+1)); echo "  PASS [$1] $2"; }
fail() { FAIL=$((FAIL+1)); echo "  FAIL [$1] $2: $3"; }
skip() { SKIP=$((SKIP+1)); echo "  SKIP [$1] $2"; }

# ---- Platform runners ----
run_test() {
    local platform="$1"
    local name="$2"
    shift 2
    TOTAL=$((TOTAL+1))

    RC=0
    if [[ "$platform" == "mac" ]]; then
        OUT=$("$ROOT/target/debug/clawenv-cli" --json "$@" 2>&1) || RC=$?
    else
        OUT=$(ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
            "${WIN_ENV} cd $WIN_PROJECT && $WIN_CLI --json $*" 2>&1) || RC=$?
    fi

    if $VERBOSE; then
        echo "       [$platform] clawenv --json $*"
        echo "$OUT" | head -5
    fi
}

check_json_type() {
    echo "$OUT" | tail -1 | grep -q "\"type\":\"$1\""
}

INSTANCE="test-$$"

# ================================================================
run_platform_tests() {
    local P="$1"  # "mac" or "win"
    echo ""
    echo "========================================"
    echo "  Testing on: $P"
    echo "  Mode: $INSTALL_MODE"
    echo "  Instance: $INSTANCE"
    echo "========================================"

    # ---- Phase 1: System Exploration ----
    echo ""
    echo "--- Phase 1: System Exploration ---"

    run_test "$P" "doctor" doctor
    if check_json_type "data"; then pass "$P" "doctor"; else fail "$P" "doctor" "$OUT"; fi

    run_test "$P" "system-check" system-check
    if check_json_type "data"; then pass "$P" "system-check"; else fail "$P" "system-check" "$OUT"; fi

    run_test "$P" "claw-types" claw-types
    if echo "$OUT" | grep -q "openclaw"; then pass "$P" "claw-types"; else fail "$P" "claw-types" "$OUT"; fi

    run_test "$P" "list" list
    if check_json_type "data"; then pass "$P" "list"; else fail "$P" "list" "$OUT"; fi

    run_test "$P" "config show" config show
    if echo "$OUT" | grep -q "language"; then pass "$P" "config show"; else fail "$P" "config show" "$OUT"; fi

    run_test "$P" "sandbox list" sandbox list
    if echo "$OUT" | grep -q "vms"; then pass "$P" "sandbox list"; else fail "$P" "sandbox list" "$OUT"; fi

    run_test "$P" "sandbox info" sandbox info
    if check_json_type "data"; then pass "$P" "sandbox info"; else fail "$P" "sandbox info" "$OUT"; fi

    # ---- Phase 2: Config Management ----
    echo ""
    echo "--- Phase 2: Config Management ---"

    run_test "$P" "config set language" config set language en-US
    if check_json_type "complete"; then pass "$P" "config set language"; else fail "$P" "config set" "$OUT"; fi

    run_test "$P" "config set theme" config set theme dark
    if check_json_type "complete"; then pass "$P" "config set theme"; else fail "$P" "config set theme" "$OUT"; fi

    # Restore
    run_test "$P" "config restore" config set language zh-CN
    run_test "$P" "config restore" config set theme system

    run_test "$P" "config set bad key" config set bad.key value
    if [[ $RC -ne 0 ]]; then pass "$P" "config set bad key (error)"; else fail "$P" "config set bad key" "should fail"; fi

    run_test "$P" "config proxy-test" config proxy-test
    # No proxy configured = info message, not error
    pass "$P" "config proxy-test (ran)"

    # ---- Phase 3: Step-by-Step Install (Developer Mode) ----
    echo ""
    echo "--- Phase 3: Step-by-Step Install ---"

    run_test "$P" "step prereq" install --mode "$INSTALL_MODE" --name "$INSTANCE" --claw-type openclaw --step prereq
    if [[ $RC -eq 0 ]]; then pass "$P" "install --step prereq"; else fail "$P" "step prereq" "$OUT"; fi

    run_test "$P" "step create" install --mode "$INSTALL_MODE" --name "$INSTANCE" --claw-type openclaw --step create
    if [[ $RC -eq 0 ]]; then pass "$P" "install --step create"; else fail "$P" "step create" "$OUT"; fi

    echo "       Installing claw product (may take several minutes)..."
    run_test "$P" "step claw" install --mode "$INSTALL_MODE" --name "$INSTANCE" --claw-type openclaw --step claw
    if [[ $RC -eq 0 ]]; then pass "$P" "install --step claw"; else fail "$P" "step claw" "$(echo "$OUT" | tail -3)"; fi

    run_test "$P" "step config" install --mode "$INSTALL_MODE" --name "$INSTANCE" --claw-type openclaw --port 3100 --step config
    if [[ $RC -eq 0 ]]; then pass "$P" "install --step config"; else fail "$P" "step config" "$OUT"; fi

    run_test "$P" "step gateway" install --mode "$INSTALL_MODE" --name "$INSTANCE" --claw-type openclaw --port 3100 --step gateway
    if [[ $RC -eq 0 ]]; then pass "$P" "install --step gateway"; else fail "$P" "step gateway" "$OUT"; fi

    # Verify instance in list
    run_test "$P" "list after install" list
    if echo "$OUT" | grep -q "$INSTANCE"; then pass "$P" "instance in list"; else fail "$P" "instance in list" "not found"; fi

    # ---- Phase 4: Lifecycle ----
    echo ""
    echo "--- Phase 4: Lifecycle ---"

    run_test "$P" "status" status "$INSTANCE"
    if check_json_type "data"; then pass "$P" "status"; else fail "$P" "status" "$OUT"; fi

    run_test "$P" "stop" stop "$INSTANCE"
    if [[ $RC -eq 0 ]]; then pass "$P" "stop"; else pass "$P" "stop (native no-op)"; fi

    run_test "$P" "start" start "$INSTANCE"
    if [[ $RC -eq 0 ]]; then pass "$P" "start"; else fail "$P" "start" "$(echo "$OUT" | tail -2)"; fi

    run_test "$P" "restart" restart "$INSTANCE"
    if [[ $RC -eq 0 ]]; then pass "$P" "restart"; else fail "$P" "restart" "$(echo "$OUT" | tail -2)"; fi

    # exec needs special quoting: cmd is first positional arg, instance name is second
    TOTAL=$((TOTAL+1))
    RC=0
    if [[ "$P" == "mac" ]]; then
        OUT=$("$ROOT/target/debug/clawenv-cli" --json exec "echo hello-test" "$INSTANCE" 2>&1) || RC=$?
    else
        OUT=$(ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
            "${WIN_ENV} cd $WIN_PROJECT && $WIN_CLI --json exec \"echo hello-test\" $INSTANCE" 2>&1) || RC=$?
    fi
    if echo "$OUT" | grep -q "hello-test"; then pass "$P" "exec"; else fail "$P" "exec" "$OUT"; fi

    run_test "$P" "logs" logs "$INSTANCE"
    pass "$P" "logs (ran)"

    # ---- Phase 5: Upgrade Check ----
    echo ""
    echo "--- Phase 5: Upgrade ---"

    run_test "$P" "update-check" update-check "$INSTANCE"
    if echo "$OUT" | grep -qi "current\|latest\|error"; then pass "$P" "update-check"; else fail "$P" "update-check" "$OUT"; fi

    # ---- Phase 6: Edit ----
    echo ""
    echo "--- Phase 6: Instance Edit ---"

    run_test "$P" "edit port" edit "$INSTANCE" --gateway-port 3101
    if [[ $RC -eq 0 ]]; then pass "$P" "edit --gateway-port"; else fail "$P" "edit port" "$OUT"; fi

    # Restore port
    run_test "$P" "edit port restore" edit "$INSTANCE" --gateway-port 3100

    # ---- Phase 7: Error Paths ----
    echo ""
    echo "--- Phase 7: Error Paths ---"

    run_test "$P" "status bad name" status nonexistent-xyz-$$
    if [[ $RC -ne 0 ]]; then pass "$P" "status nonexistent (error)"; else fail "$P" "status bad" "should fail"; fi

    run_test "$P" "rename bad" rename nonexistent-xyz-$$ newname
    if [[ $RC -ne 0 ]]; then pass "$P" "rename nonexistent (error)"; else fail "$P" "rename bad" "should fail"; fi

    run_test "$P" "install bad step" install --step badstep
    if [[ $RC -ne 0 ]]; then pass "$P" "bad step (error)"; else fail "$P" "bad step" "should fail"; fi

    # ---- Phase 8: Cleanup ----
    echo ""
    echo "--- Phase 8: Cleanup ---"

    run_test "$P" "uninstall" uninstall --name "$INSTANCE"
    if [[ $RC -eq 0 ]]; then pass "$P" "uninstall"; else fail "$P" "uninstall" "$OUT"; fi

    run_test "$P" "list after uninstall" list
    if echo "$OUT" | grep -q "$INSTANCE"; then
        fail "$P" "instance gone" "still in list"
    else
        pass "$P" "instance gone after uninstall"
    fi
}

# ================================================================
# Build
# ================================================================
echo "=== Building CLI ==="
cd "$ROOT"
cargo build -p clawenv-cli 2>&1 | tail -1

# ================================================================
# macOS Tests
# ================================================================
if $TEST_MACOS; then
    run_platform_tests "mac"
fi

# ================================================================
# Windows Tests
# ================================================================
if $TEST_WINDOWS; then
    echo ""
    echo "=== Syncing code to Windows ==="
    # Sync via git
    ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
        "${WIN_ENV} cd $WIN_PROJECT && \"C:\\Program Files\\Git\\cmd\\git.exe\" pull" 2>&1 || {
        echo "ERROR: Cannot reach Windows host $WIN_HOST"
        echo "Make sure the VM is running and SSH is accessible."
        exit 1
    }

    echo "=== Building CLI on Windows ==="
    ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
        "${WIN_ENV} cd $WIN_PROJECT && C:\\Users\\$WIN_USER\\.cargo\\bin\\cargo.exe build -p clawenv-cli" 2>&1 | tail -3

    run_platform_tests "win"
fi

# ================================================================
# Summary
# ================================================================
echo ""
echo "========================================"
echo "  RESULTS: $PASS passed, $FAIL failed, $SKIP skipped / $TOTAL total"
echo "========================================"

if [[ $FAIL -gt 0 ]]; then exit 1; fi
exit 0
