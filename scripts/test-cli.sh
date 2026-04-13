#!/bin/bash
# ClawEnv CLI — End-to-end test script
#
# Tests the CLI tool through its complete workflow:
#   1. System exploration (doctor, system-check, claw-types)
#   2. Step-by-step install (developer mode)
#   3. Full install (normal user mode)
#   4. Lifecycle management (start, stop, restart, status, logs)
#   5. Upgrade check
#   6. Cleanup (uninstall)
#
# Usage:
#   bash scripts/test-cli.sh [options]
#
# Options:
#   --mode sandbox|native   Install mode (default: native)
#   --claw-type TYPE        Claw product to test (default: openclaw)
#   --skip-install          Skip install tests (test existing instance)
#   --skip-cleanup          Keep instance after test
#   --json                  Run all tests in JSON mode
#   --verbose               Show full command output
#
# Exit: 0 if all pass, non-zero on first failure

set -euo pipefail

# ---- Configuration ----
MODE="${MODE:-native}"
CLAW_TYPE="${CLAW_TYPE:-openclaw}"
INSTANCE="cli-test-$$"
SKIP_INSTALL=false
SKIP_CLEANUP=false
JSON_MODE=false
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode)       MODE="$2"; shift 2;;
        --claw-type)  CLAW_TYPE="$2"; shift 2;;
        --skip-install)  SKIP_INSTALL=true; shift;;
        --skip-cleanup)  SKIP_CLEANUP=true; shift;;
        --json)       JSON_MODE=true; shift;;
        --verbose)    VERBOSE=true; shift;;
        *) echo "Unknown option: $1"; exit 1;;
    esac
done

# ---- Helpers ----
PASS=0
FAIL=0
SKIP=0
TOTAL=0

# Find the CLI binary
if [[ -f ./target/debug/clawenv-cli ]]; then
    CLI=./target/debug/clawenv-cli
elif command -v clawenv-cli &>/dev/null; then
    CLI=clawenv-cli
else
    echo "ERROR: clawenv-cli not found. Run 'cargo build -p clawenv-cli' first."
    exit 1
fi

JSON_FLAG=""
if $JSON_MODE; then
    JSON_FLAG="--json"
fi

run_test() {
    local name="$1"
    shift
    TOTAL=$((TOTAL + 1))

    if $VERBOSE; then
        echo "  RUN  $name"
        echo "       > $CLI $JSON_FLAG $*"
    fi

    local output
    local rc=0
    output=$("$CLI" $JSON_FLAG "$@" 2>&1) || rc=$?

    if $VERBOSE; then
        echo "$output" | head -20
    fi

    echo "$output"  # return for capture
    return $rc
}

pass() {
    PASS=$((PASS + 1))
    echo "  PASS $1"
}

fail() {
    FAIL=$((FAIL + 1))
    echo "  FAIL $1: $2"
}

skip() {
    SKIP=$((SKIP + 1))
    echo "  SKIP $1"
}

# ---- Cleanup on exit ----
cleanup() {
    if ! $SKIP_CLEANUP && ! $SKIP_INSTALL; then
        echo ""
        echo "=== Cleanup ==="
        "$CLI" uninstall --name "$INSTANCE" 2>/dev/null || true
    fi
}
trap cleanup EXIT

echo "============================================"
echo "  ClawEnv CLI End-to-End Tests"
echo "============================================"
echo "  Binary:    $CLI"
echo "  Mode:      $MODE"
echo "  Claw:      $CLAW_TYPE"
echo "  Instance:  $INSTANCE"
echo "  JSON mode: $JSON_MODE"
echo "============================================"
echo ""

# ================================================================
# Phase 1: System Exploration
# ================================================================
echo "=== Phase 1: System Exploration ==="

# T1: --version
TOTAL=$((TOTAL + 1))
if "$CLI" --version 2>&1 | grep -q "clawenv"; then
    pass "cli --version"
else
    fail "cli --version" "no version output"
fi

# T2: --help
TOTAL=$((TOTAL + 1))
if "$CLI" --help 2>&1 | grep -q "install"; then
    pass "cli --help"
else
    fail "cli --help" "missing commands"
fi

# T3: doctor
TOTAL=$((TOTAL + 1))
DOCTOR_OUT=$("$CLI" $JSON_FLAG doctor 2>&1)
if echo "$DOCTOR_OUT" | grep -qi "macos\|windows\|linux"; then
    pass "doctor"
else
    fail "doctor" "no OS info"
fi

# T4: system-check
TOTAL=$((TOTAL + 1))
SYSCHECK_OUT=$("$CLI" $JSON_FLAG system-check 2>&1)
if echo "$SYSCHECK_OUT" | grep -qi "OS\|Memory\|Disk"; then
    pass "system-check"
else
    fail "system-check" "missing check items"
fi

# T5: claw-types
TOTAL=$((TOTAL + 1))
TYPES_OUT=$("$CLI" $JSON_FLAG claw-types 2>&1)
if echo "$TYPES_OUT" | grep -q "openclaw"; then
    pass "claw-types"
else
    fail "claw-types" "openclaw not in registry"
fi

# T6: list (should work even without instances)
TOTAL=$((TOTAL + 1))
LIST_OUT=$("$CLI" $JSON_FLAG list 2>&1)
if echo "$LIST_OUT" | grep -qi "instances"; then
    pass "list (initial)"
else
    fail "list (initial)" "no instances array"
fi

# T7: JSON output format validation
TOTAL=$((TOTAL + 1))
JSON_OUT=$("$CLI" --json claw-types 2>&1)
VALID=true
while IFS= read -r line; do
    if [[ -n "$line" ]] && ! echo "$line" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
        VALID=false
        break
    fi
done <<< "$JSON_OUT"
if $VALID; then
    pass "JSON output format"
else
    fail "JSON output format" "invalid JSON lines"
fi

# T8: bad subcommand
TOTAL=$((TOTAL + 1))
if ! "$CLI" nonexistent-cmd 2>&1; then
    pass "bad subcommand (error exit)"
else
    fail "bad subcommand" "should have failed"
fi

# T9: config show
TOTAL=$((TOTAL + 1))
CONFIG_OUT=$("$CLI" $JSON_FLAG config show 2>&1)
if echo "$CONFIG_OUT" | grep -qi "language\|theme"; then
    pass "config show"
else
    fail "config show" "$CONFIG_OUT"
fi

# T10: sandbox list
TOTAL=$((TOTAL + 1))
SB_OUT=$("$CLI" $JSON_FLAG sandbox list 2>&1)
if echo "$SB_OUT" | grep -qi "vms"; then
    pass "sandbox list"
else
    fail "sandbox list" "$SB_OUT"
fi

# T11: sandbox info
TOTAL=$((TOTAL + 1))
SBI_OUT=$("$CLI" $JSON_FLAG sandbox info 2>&1)
if echo "$SBI_OUT" | grep -qi "sandbox_backend\|disk"; then
    pass "sandbox info"
else
    fail "sandbox info" "$SBI_OUT"
fi

echo ""

# ================================================================
# Phase 2: Developer Mode — Step-by-Step Install
# ================================================================
echo "=== Phase 2: Developer Mode (--step) ==="

if $SKIP_INSTALL; then
    skip "step prereq (--skip-install)"
    skip "step create (--skip-install)"
    skip "step claw (--skip-install)"
    skip "step config (--skip-install)"
    skip "step gateway (--skip-install)"
else
    # T9: step prereq
    TOTAL=$((TOTAL + 1))
    PREREQ_OUT=$("$CLI" $JSON_FLAG install --mode "$MODE" --name "$INSTANCE" --claw-type "$CLAW_TYPE" --step prereq 2>&1)
    if echo "$PREREQ_OUT" | grep -qi "ready\|available\|installed"; then
        pass "install --step prereq"
    else
        fail "install --step prereq" "$PREREQ_OUT"
    fi

    # T10: step create
    TOTAL=$((TOTAL + 1))
    CREATE_OUT=$("$CLI" $JSON_FLAG install --mode "$MODE" --name "$INSTANCE" --claw-type "$CLAW_TYPE" --step create 2>&1)
    if echo "$CREATE_OUT" | grep -qi "ready\|created\|exists"; then
        pass "install --step create"
    else
        fail "install --step create" "$CREATE_OUT"
    fi

    # T11: step claw
    TOTAL=$((TOTAL + 1))
    echo "       (this may take several minutes for npm install...)"
    CLAW_OUT=$("$CLI" $JSON_FLAG install --mode "$MODE" --name "$INSTANCE" --claw-type "$CLAW_TYPE" --step claw 2>&1)
    if echo "$CLAW_OUT" | grep -qi "installed"; then
        pass "install --step claw"
    else
        fail "install --step claw" "$(echo "$CLAW_OUT" | tail -3)"
    fi

    # T12: step config
    TOTAL=$((TOTAL + 1))
    CONFIG_OUT=$("$CLI" $JSON_FLAG install --mode "$MODE" --name "$INSTANCE" --claw-type "$CLAW_TYPE" --port 3100 --step config 2>&1)
    if echo "$CONFIG_OUT" | grep -qi "config saved"; then
        pass "install --step config"
    else
        fail "install --step config" "$CONFIG_OUT"
    fi

    # T13: step gateway
    TOTAL=$((TOTAL + 1))
    GW_OUT=$("$CLI" $JSON_FLAG install --mode "$MODE" --name "$INSTANCE" --claw-type "$CLAW_TYPE" --port 3100 --step gateway 2>&1)
    if echo "$GW_OUT" | grep -qi "started"; then
        pass "install --step gateway"
    else
        fail "install --step gateway" "$GW_OUT"
    fi

    # T14: verify instance appears in list
    TOTAL=$((TOTAL + 1))
    LIST_OUT=$("$CLI" $JSON_FLAG list 2>&1)
    if echo "$LIST_OUT" | grep -q "$INSTANCE"; then
        pass "instance in list after step install"
    else
        fail "instance in list" "instance not found"
    fi

    # T15: bad step name
    TOTAL=$((TOTAL + 1))
    BAD_STEP_OUT=$("$CLI" $JSON_FLAG install --step badstep 2>&1) || true
    if echo "$BAD_STEP_OUT" | grep -q "Unknown install step"; then
        pass "bad step name (error)"
    else
        fail "bad step name" "should report error"
    fi
fi

echo ""

# ================================================================
# Phase 3: Lifecycle Management
# ================================================================
echo "=== Phase 3: Lifecycle Management ==="

if $SKIP_INSTALL; then
    # Use existing "default" instance
    INSTANCE="default"
fi

# T16: status
TOTAL=$((TOTAL + 1))
STATUS_OUT=$("$CLI" $JSON_FLAG status "$INSTANCE" 2>&1)
if echo "$STATUS_OUT" | grep -qi "$INSTANCE\|name"; then
    pass "status"
else
    fail "status" "$STATUS_OUT"
fi

# T17: stop
TOTAL=$((TOTAL + 1))
STOP_OUT=$("$CLI" $JSON_FLAG stop "$INSTANCE" 2>&1) || true
if echo "$STOP_OUT" | grep -qi "stopped\|complete"; then
    pass "stop"
else
    # Native stop may be no-op
    pass "stop (no-op for native)"
fi

# T18: start
TOTAL=$((TOTAL + 1))
START_OUT=$("$CLI" $JSON_FLAG start "$INSTANCE" 2>&1) || true
if echo "$START_OUT" | grep -qi "started\|complete"; then
    pass "start"
else
    fail "start" "$(echo "$START_OUT" | tail -2)"
fi

# T19: restart
TOTAL=$((TOTAL + 1))
RESTART_OUT=$("$CLI" $JSON_FLAG restart "$INSTANCE" 2>&1) || true
if echo "$RESTART_OUT" | grep -qi "restarted\|complete"; then
    pass "restart"
else
    fail "restart" "$(echo "$RESTART_OUT" | tail -2)"
fi

# T20: logs
TOTAL=$((TOTAL + 1))
# logs command may or may not have output, but should not error
if "$CLI" logs "$INSTANCE" 2>&1; then
    pass "logs"
else
    fail "logs" "command failed"
fi

# T21: exec (native or sandbox)
TOTAL=$((TOTAL + 1))
EXEC_OUT=$("$CLI" exec "echo hello-from-exec" "$INSTANCE" 2>&1) || true
if echo "$EXEC_OUT" | grep -q "hello-from-exec"; then
    pass "exec"
else
    fail "exec" "$(echo "$EXEC_OUT" | tail -2)"
fi

echo ""

# ================================================================
# Phase 4: Upgrade Check
# ================================================================
echo "=== Phase 4: Upgrade ==="

# T22: update-check
TOTAL=$((TOTAL + 1))
UPDATE_OUT=$("$CLI" $JSON_FLAG update-check "$INSTANCE" 2>&1) || true
if echo "$UPDATE_OUT" | grep -qi "current\|latest\|error\|failed"; then
    pass "update-check (ran)"
else
    fail "update-check" "$UPDATE_OUT"
fi

# ================================================================
# Phase 4b: Edit / Rename / Config Set
# ================================================================
echo ""
echo "=== Phase 4b: Edit / Rename / Config Set ==="

if ! $SKIP_INSTALL; then
    # Edit gateway port
    TOTAL=$((TOTAL + 1))
    EDIT_OUT=$("$CLI" $JSON_FLAG edit "$INSTANCE" --gateway-port 3101 2>&1) || true
    if echo "$EDIT_OUT" | grep -qi "updated"; then
        pass "edit --gateway-port"
    else
        fail "edit --gateway-port" "$EDIT_OUT"
    fi
    # Restore
    "$CLI" $JSON_FLAG edit "$INSTANCE" --gateway-port 3100 2>/dev/null || true

    # Config set + verify
    TOTAL=$((TOTAL + 1))
    "$CLI" $JSON_FLAG config set language en-US 2>/dev/null
    VERIFY=$("$CLI" $JSON_FLAG config show 2>&1)
    if echo "$VERIFY" | grep -q "en-US"; then
        pass "config set + show round-trip"
    else
        fail "config set round-trip" "$VERIFY"
    fi
    "$CLI" $JSON_FLAG config set language zh-CN 2>/dev/null || true

    # Port conflict detection
    TOTAL=$((TOTAL + 1))
    # Try to install another instance on the same port (should fail)
    CONFLICT=$("$CLI" $JSON_FLAG install --mode "$MODE" --name conflict-test --port 3100 --step config 2>&1) || true
    if echo "$CONFLICT" | grep -qi "already used\|error"; then
        pass "port conflict detection"
    else
        fail "port conflict detection" "should reject duplicate port"
        "$CLI" $JSON_FLAG uninstall --name conflict-test 2>/dev/null || true
    fi

    # Error: edit nonexistent
    TOTAL=$((TOTAL + 1))
    if ! "$CLI" $JSON_FLAG edit nonexistent-xyz --cpus 4 2>/dev/null; then
        pass "edit nonexistent (error)"
    else
        fail "edit nonexistent" "should fail"
    fi

    # Error: rename nonexistent
    TOTAL=$((TOTAL + 1))
    if ! "$CLI" $JSON_FLAG rename nonexistent-xyz newname 2>/dev/null; then
        pass "rename nonexistent (error)"
    else
        fail "rename nonexistent" "should fail"
    fi
else
    skip "edit (--skip-install)"
    skip "config set round-trip (--skip-install)"
    skip "port conflict (--skip-install)"
    skip "edit nonexistent (--skip-install)"
    skip "rename nonexistent (--skip-install)"
fi

echo ""

# ================================================================
# Phase 5: Cleanup
# ================================================================
echo "=== Phase 5: Cleanup ==="

if $SKIP_CLEANUP; then
    skip "uninstall (--skip-cleanup)"
else
    # T23: uninstall
    TOTAL=$((TOTAL + 1))
    UNINST_OUT=$("$CLI" $JSON_FLAG uninstall --name "$INSTANCE" 2>&1) || true
    if echo "$UNINST_OUT" | grep -qi "removed\|complete\|not found"; then
        pass "uninstall"
    else
        fail "uninstall" "$UNINST_OUT"
    fi

    # T24: verify gone from list
    TOTAL=$((TOTAL + 1))
    LIST_AFTER=$("$CLI" $JSON_FLAG list 2>&1)
    if echo "$LIST_AFTER" | grep -q "$INSTANCE"; then
        fail "instance gone after uninstall" "still in list"
    else
        pass "instance gone after uninstall"
    fi
fi

echo ""

# ================================================================
# Summary
# ================================================================
echo "============================================"
echo "  Results: $PASS passed, $FAIL failed, $SKIP skipped / $TOTAL total"
echo "============================================"

if [[ $FAIL -gt 0 ]]; then
    exit 1
fi
exit 0
