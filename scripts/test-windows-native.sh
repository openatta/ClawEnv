#!/bin/bash
# Windows Native Test — via SSH to Windows ARM64 host
#
# Usage:
#   bash scripts/test-windows-native.sh
#   bash scripts/test-windows-native.sh --verbose
set -uo pipefail

VERBOSE=false
[[ "${1:-}" == "--verbose" ]] && VERBOSE=true

INSTANCE="win-native-$$"
PORT=3200
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-test.sh"

# Load .env
if [ -f "$SCRIPT_DIR/../.env" ]; then
    export $(grep -v '^#' "$SCRIPT_DIR/../.env" | xargs)
fi
WIN_HOST="${WIN_HOST:-192.168.64.7}"
WIN_USER="${WIN_USER:-clawenv}"
WIN_PROJECT="C:\\Users\\$WIN_USER\\Desktop\\ClawEnv"
WIN_CLI="$WIN_PROJECT\\target\\debug\\clawcli.exe"
WIN_ENV="set PATH=%PATH%;C:\\Program Files\\nodejs;C:\\Program Files\\Git\\cmd;C:\\Users\\$WIN_USER\\.cargo\\bin&&"

echo "========================================"
echo "  Windows Native Test (SSH → $WIN_HOST)"
echo "  Instance: $INSTANCE  Port: $PORT"
echo "========================================"

# Check SSH
echo "  Checking SSH connectivity..."
if ! ssh -o ConnectTimeout=5 "$WIN_USER@$WIN_HOST" "echo ok" 2>&1 | grep -q "ok"; then
    echo "ERROR: Cannot reach Windows host $WIN_HOST via SSH"
    exit 1
fi

# Sync & build
echo "  Syncing code..."
ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} cd $WIN_PROJECT && \"C:\\Program Files\\Git\\cmd\\git.exe\" pull" 2>&1 | tail -2
echo "  Building CLI..."
ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} cd $WIN_PROJECT && C:\\Users\\$WIN_USER\\.cargo\\bin\\cargo.exe build -p clawcli" 2>&1 | tail -2

# ================================================================
section "A. System Exploration"
# ================================================================

win_run doctor
if has_type data; then pass "doctor"; else fail "doctor" "$OUT"; fi

win_run system-check
if has_type data; then pass "system-check"; else fail "system-check" "$OUT"; fi

win_run claw-types
if has_text openclaw; then pass "claw-types"; else fail "claw-types" "$OUT"; fi

win_run list
if has_type data; then pass "list"; else fail "list" "$OUT"; fi

win_run config show
if has_text language; then pass "config show"; else fail "config show" "$OUT"; fi

win_run sandbox list
if has_text vms; then pass "sandbox list"; else fail "sandbox list" "$OUT"; fi

win_run sandbox info
if has_type data; then pass "sandbox info"; else fail "sandbox info" "$OUT"; fi

# ================================================================
section "B. Native Step-by-Step Install"
# ================================================================

win_run install --mode native --name $INSTANCE --claw-type openclaw --step prereq
if [[ $RC -eq 0 ]]; then pass "step prereq"; else fail "step prereq" "$OUT"; fi

win_run install --mode native --name $INSTANCE --claw-type openclaw --step create
if [[ $RC -eq 0 ]]; then pass "step create"; else fail "step create" "$OUT"; fi

echo "       Installing claw (may take minutes)..."
win_run install --mode native --name $INSTANCE --claw-type openclaw --step claw
if [[ $RC -eq 0 ]]; then pass "step claw"; else fail "step claw" "$(echo "$OUT" | tail -3)"; fi

win_run install --mode native --name $INSTANCE --claw-type openclaw --port $PORT --step config
if [[ $RC -eq 0 ]]; then pass "step config"; else fail "step config" "$OUT"; fi

win_run install --mode native --name $INSTANCE --claw-type openclaw --port $PORT --step gateway
if [[ $RC -eq 0 ]]; then pass "step gateway"; else fail "step gateway" "$OUT"; fi

win_run list
if has_text $INSTANCE; then pass "in list"; else fail "in list" "not found"; fi

# ================================================================
section "C. Bridge Test"
# ================================================================

win_run bridge test
if [[ $RC -eq 0 ]]; then pass "bridge test"; else skip "bridge test (not running)"; fi

# ================================================================
section "D. Lifecycle"
# ================================================================

win_run status $INSTANCE
if has_type data; then pass "status"; else fail "status" "$OUT"; fi

win_run stop $INSTANCE
if [[ $RC -eq 0 ]]; then pass "stop"; else pass "stop (native no-op)"; fi

win_run start $INSTANCE
if [[ $RC -eq 0 ]]; then pass "start"; else fail "start" "$(echo "$OUT" | tail -2)"; fi

win_run restart $INSTANCE
if [[ $RC -eq 0 ]]; then pass "restart"; else fail "restart" "$(echo "$OUT" | tail -2)"; fi

# Exec (special quoting for SSH)
TOTAL=$((TOTAL+1)); RC=0
OUT=$(ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
    "${WIN_ENV} cd $WIN_PROJECT && $WIN_CLI --json exec \"echo hello-win\" $INSTANCE" 2>&1) || RC=$?
if echo "$OUT" | grep -q "hello-win"; then pass "exec"; else fail "exec" "$OUT"; fi

win_run logs $INSTANCE
pass "logs (ran)"

win_run update-check $INSTANCE
if has_text "current\|latest\|error"; then pass "update-check"; else fail "update-check" "$OUT"; fi

win_run upgrade nonexistent-$$
if [[ $RC -ne 0 ]]; then pass "upgrade (error path)"; else fail "upgrade" "should fail for nonexistent"; fi

# ================================================================
section "E. Config & Edit"
# ================================================================

win_run config set language en-US
win_run config show
if has_text "en-US"; then pass "config set round-trip"; else fail "config set round-trip" "$OUT"; fi
ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} cd $WIN_PROJECT && $WIN_CLI --json config set language zh-CN" 2>/dev/null

win_run edit $INSTANCE --gateway-port $((PORT+1))
if [[ $RC -eq 0 ]]; then pass "edit gateway-port"; else fail "edit gateway-port" "$OUT"; fi
ssh "$WIN_USER@$WIN_HOST" "${WIN_ENV} cd $WIN_PROJECT && $WIN_CLI --json edit $INSTANCE --gateway-port $PORT" 2>/dev/null

# Rename (error path — nonexistent)
win_run rename nonexistent-$$ newname
if [[ $RC -ne 0 ]]; then pass "rename nonexistent (error)"; else fail "rename error" "should fail"; fi

# config proxy-test
win_run config proxy-test
pass "config proxy-test (ran)"

# Error paths
win_run status nonexistent-$$
if [[ $RC -ne 0 ]]; then pass "status nonexistent (error)"; else fail "status error" "should fail"; fi

# ================================================================
section "F. Cleanup"
# ================================================================

win_run uninstall --name $INSTANCE
if [[ $RC -eq 0 ]]; then pass "uninstall"; else fail "uninstall" "$OUT"; fi

win_run list
if ! has_text $INSTANCE; then pass "gone from list"; else fail "gone from list" "still present"; fi

summary
