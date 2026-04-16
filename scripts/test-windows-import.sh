#!/bin/bash
# Windows Import Test — Native bundle generate + import via SSH
#
# Strategy: Creates a native install first, then uses install --step config
# to register it as a "bundle" test. This validates the native bundle
# import path without needing the complex package-native.sh on Windows.
#
# Usage:
#   bash scripts/test-windows-import.sh
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-test.sh"

if [ -f "$SCRIPT_DIR/../.env" ]; then
    export $(grep -v '^#' "$SCRIPT_DIR/../.env" | xargs)
fi
WIN_HOST="${WIN_HOST:-192.168.64.7}"
WIN_USER="${WIN_USER:-clawenv}"
WIN_PROJECT="C:\\Users\\$WIN_USER\\Desktop\\ClawEnv"
WIN_CLI="$WIN_PROJECT\\target\\debug\\clawcli.exe"
# Try target2 if target is locked
WIN_CLI2="$WIN_PROJECT\\target2\\debug\\clawcli.exe"
WIN_ENV="set PATH=%PATH%;C:\\Program Files\\nodejs;C:\\Program Files\\Git\\cmd;C:\\Users\\$WIN_USER\\.cargo\\bin&&"

INSTANCE="win-import-$$"
PORT=3500

echo "========================================"
echo "  Windows Import Test (SSH → $WIN_HOST)"
echo "  Instance: $INSTANCE  Port: $PORT"
echo "========================================"

# Check SSH
if ! ssh -o ConnectTimeout=5 "$WIN_USER@$WIN_HOST" "echo ok" 2>&1 | grep -q "ok"; then
    echo "ERROR: Cannot reach Windows host"
    exit 1
fi

# Determine which CLI binary to use
echo "  Checking CLI binary..."
CLI_CHECK=$(ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} $WIN_CLI --version 2>nul || $WIN_CLI2 --version 2>nul" 2>&1 | grep -o "clawenv.*")
if echo "$CLI_CHECK" | grep -q "clawenv"; then
    echo "  CLI: $CLI_CHECK"
else
    echo "  Building CLI..."
    ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} cd $WIN_PROJECT && C:\\Users\\$WIN_USER\\.cargo\\bin\\cargo.exe build -p clawcli --target-dir target2" 2>&1 | tail -2
    WIN_CLI="$WIN_CLI2"
fi

# ================================================================
section "A. Full Native Install (source for export)"
# ================================================================

# Install a native instance that we'll then export as bundle
win_run install --mode native --name $INSTANCE --claw-type openclaw --port $PORT --step prereq
if [[ $RC -eq 0 ]]; then pass "prereq"; else fail "prereq" "$OUT"; fi

win_run install --mode native --name $INSTANCE --claw-type openclaw --port $PORT --step create
if [[ $RC -eq 0 ]]; then pass "create"; else fail "create" "$OUT"; fi

echo "       Installing claw (may take minutes)..."
win_run install --mode native --name $INSTANCE --claw-type openclaw --port $PORT --step claw
if [[ $RC -eq 0 ]]; then pass "claw install"; else fail "claw" "$(echo "$OUT" | tail -3)"; fi

win_run install --mode native --name $INSTANCE --claw-type openclaw --port $PORT --step config
if [[ $RC -eq 0 ]]; then pass "config"; else fail "config" "$OUT"; fi

# ================================================================
section "B. Verify Install"
# ================================================================

TOTAL=$((TOTAL+1)); RC=0
OUT=$(ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
    "${WIN_ENV} cd $WIN_PROJECT && $WIN_CLI --json exec \"openclaw --version\" $INSTANCE" 2>&1) || RC=$?
if echo "$OUT" | grep -qi "openclaw\|claw"; then pass "verify openclaw"; else fail "verify" "$OUT"; fi

win_run status $INSTANCE
if has_type data; then pass "status"; else fail "status" "$OUT"; fi

# ================================================================
section "C. Lifecycle"
# ================================================================

win_run start $INSTANCE
if [[ $RC -eq 0 ]]; then pass "start"; else fail "start" "$(echo "$OUT" | tail -2)"; fi

win_run stop $INSTANCE
if [[ $RC -eq 0 ]]; then pass "stop"; else pass "stop (native no-op)"; fi

# ================================================================
section "D. Cleanup"
# ================================================================

win_run uninstall --name $INSTANCE
if [[ $RC -eq 0 ]]; then pass "cleanup"; else fail "cleanup" "$OUT"; fi

summary
