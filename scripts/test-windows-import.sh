#!/bin/bash
# Windows Import Test — Native bundle import via SSH
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
WIN_CLI="$WIN_PROJECT\\target\\debug\\clawenv-cli.exe"
WIN_ENV="set PATH=%PATH%;C:\\Program Files\\nodejs;C:\\Program Files\\Git\\cmd;C:\\Users\\$WIN_USER\\.cargo\\bin&&"

INSTANCE="win-bundle-$$"
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

# ================================================================
section "A. Generate Native Bundle on Windows"
# ================================================================

TOTAL=$((TOTAL+1))
echo "       Generating bundle on Windows (5-10 min)..."
BUNDLE_RC=0
ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} cd $WIN_PROJECT && bash tools/package-native.sh openclaw latest test-bundle-import" 2>&1 | tail -5 || BUNDLE_RC=$?

# Find the bundle file
BUNDLE_FILE=$(ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} cd $WIN_PROJECT && dir /b test-bundle-import\\clawenv-native-*.tar.gz 2>nul" 2>&1 | grep ".tar.gz" | head -1 | tr -d '\r')

if [[ $BUNDLE_RC -eq 0 ]] && [[ -n "$BUNDLE_FILE" ]]; then
    pass "bundle generate"
else
    fail "bundle generate" "package-native.sh failed (rc=$BUNDLE_RC)"
    echo "       Skipping remaining import tests"
    summary
fi

# ================================================================
section "B. Import Bundle"
# ================================================================

win_run install --mode native --name $INSTANCE --image "test-bundle-import\\$BUNDLE_FILE" --port $PORT
if [[ $RC -eq 0 ]]; then pass "bundle import"; else fail "bundle import" "$(echo "$OUT" | tail -3)"; fi

# ================================================================
section "C. Verify"
# ================================================================

TOTAL=$((TOTAL+1)); RC=0
OUT=$(ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
    "${WIN_ENV} cd $WIN_PROJECT && $WIN_CLI --json exec \"openclaw --version\" $INSTANCE" 2>&1) || RC=$?
if echo "$OUT" | grep -qi "openclaw\|claw"; then pass "bundle verify"; else fail "bundle verify" "$OUT"; fi

win_run status $INSTANCE
if has_type data; then pass "status"; else fail "status" "$OUT"; fi

# ================================================================
section "D. Cleanup"
# ================================================================

win_run uninstall --name $INSTANCE
if [[ $RC -eq 0 ]]; then pass "cleanup uninstall"; else fail "cleanup" "$OUT"; fi

# Remove bundle dir
ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} cd $WIN_PROJECT && rmdir /s /q test-bundle-import 2>nul" 2>/dev/null

summary
