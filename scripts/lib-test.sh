#!/bin/bash
# Shared test helpers for all platform test scripts.
# Source this file: source "$(dirname "$0")/lib-test.sh"

PASS=0; FAIL=0; SKIP=0; TOTAL=0
OUT=""; RC=0

# Find CLI binary
find_cli() {
    if [[ -f ./target/debug/clawenv-cli ]]; then
        CLI=./target/debug/clawenv-cli
    elif command -v clawenv-cli &>/dev/null; then
        CLI=clawenv-cli
    else
        echo "ERROR: clawenv-cli not found. Run 'cargo build -p clawenv-cli' first."
        exit 1
    fi
}

pass() { PASS=$((PASS+1)); echo "  PASS $1"; }
fail() { FAIL=$((FAIL+1)); echo "  FAIL $1: $2"; }
skip() { SKIP=$((SKIP+1)); TOTAL=$((TOTAL+1)); echo "  SKIP $1"; }

# Run a CLI command locally, capture output + exit code
run() {
    TOTAL=$((TOTAL+1))
    RC=0
    OUT=$("$CLI" --json "$@" 2>&1) || RC=$?
}

# Run a CLI command via SSH on Windows
win_run() {
    TOTAL=$((TOTAL+1))
    RC=0
    OUT=$(ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
        "${WIN_ENV} cd $WIN_PROJECT && $WIN_CLI --json $*" 2>&1) || RC=$?
}

# Check if last output contains a JSON type
has_type() { echo "$OUT" | grep -q "\"type\":\"$1\""; }
has_text() { echo "$OUT" | grep -qi "$1"; }

# Print summary and exit
summary() {
    echo ""
    echo "========================================"
    echo "  RESULTS: $PASS passed, $FAIL failed, $SKIP skipped / $TOTAL total"
    echo "========================================"
    [[ $FAIL -gt 0 ]] && exit 1
    exit 0
}

# Section header
section() {
    echo ""
    echo "--- $1 ---"
}
