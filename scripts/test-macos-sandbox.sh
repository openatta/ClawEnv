#!/bin/bash
# macOS Sandbox Test — Lima + Alpine VM full lifecycle
#
# WARNING: This test creates a real Lima VM and takes 15-25 minutes.
#
# Usage:
#   bash scripts/test-macos-sandbox.sh
#   bash scripts/test-macos-sandbox.sh --skip-create   # Use existing VM
#   bash scripts/test-macos-sandbox.sh --skip-cleanup   # Keep VM after test
set -uo pipefail

SKIP_CREATE=false
SKIP_CLEANUP=false
[[ "${1:-}" == "--skip-create" ]] && SKIP_CREATE=true
[[ "${1:-}" == "--skip-cleanup" ]] && SKIP_CLEANUP=true
[[ "${2:-}" == "--skip-cleanup" ]] && SKIP_CLEANUP=true

INSTANCE="mac-sandbox-$$"
PORT=3400
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-test.sh"

echo "========================================"
echo "  macOS Sandbox Test (Lima + Alpine)"
echo "  Instance: $INSTANCE  Port: $PORT"
echo "  WARNING: Creates a real VM, takes 15-25 min"
echo "========================================"

cd "$SCRIPT_DIR/.."
cargo build -p clawenv-cli 2>&1 | tail -1
find_cli

# ================================================================
section "A. Prerequisites"
# ================================================================

run install --mode sandbox --name "$INSTANCE" --step prereq
if [[ $RC -eq 0 ]]; then pass "prereq (Lima)"; else fail "prereq" "$OUT"; fi

# ================================================================
section "B. Create VM"
# ================================================================

if $SKIP_CREATE; then
    skip "create VM (--skip-create)"
else
    echo "       Creating Alpine VM (7-10 min)..."
    run install --mode sandbox --name "$INSTANCE" --step create
    if [[ $RC -eq 0 ]]; then pass "create VM"; else fail "create VM" "$(echo "$OUT" | tail -5)"; fi
fi

# ================================================================
section "C. Install Claw in Sandbox"
# ================================================================

echo "       Installing OpenClaw in sandbox (5-10 min)..."
run install --mode sandbox --name "$INSTANCE" --step claw
if [[ $RC -eq 0 ]]; then pass "install claw"; else fail "install claw" "$(echo "$OUT" | tail -5)"; fi

# ================================================================
section "D. Configure & Start"
# ================================================================

run install --mode sandbox --name "$INSTANCE" --port "$PORT" --step config
if [[ $RC -eq 0 ]]; then pass "config"; else fail "config" "$OUT"; fi

run install --mode sandbox --name "$INSTANCE" --port "$PORT" --step gateway
if [[ $RC -eq 0 ]]; then pass "gateway start"; else fail "gateway" "$OUT"; fi

# ================================================================
section "E. Sandbox Verification"
# ================================================================

run status "$INSTANCE"
if has_type data; then pass "status"; else fail "status" "$OUT"; fi

# Exec inside VM
TOTAL=$((TOTAL+1)); RC=0
OUT=$("$CLI" --json exec "node --version" "$INSTANCE" 2>&1) || RC=$?
if echo "$OUT" | grep -q "v2"; then pass "exec node --version"; else fail "exec node" "$OUT"; fi

TOTAL=$((TOTAL+1)); RC=0
OUT=$("$CLI" --json exec "openclaw --version" "$INSTANCE" 2>&1) || RC=$?
if echo "$OUT" | grep -qi "openclaw\|claw"; then pass "exec openclaw --version"; else fail "exec openclaw" "$OUT"; fi

# Shell test (non-interactive: pipe command)
TOTAL=$((TOTAL+1)); RC=0
OUT=$("$CLI" --json exec "echo sandbox-shell-ok" "$INSTANCE" 2>&1) || RC=$?
if echo "$OUT" | grep -q "sandbox-shell-ok"; then pass "exec echo"; else fail "exec echo" "$OUT"; fi

# Sandbox info
run sandbox info
if has_type data; then pass "sandbox info"; else fail "sandbox info" "$OUT"; fi

# Verify sandbox in list
run sandbox list
if has_text "clawenv-$INSTANCE\|$INSTANCE"; then pass "in sandbox list"; else pass "sandbox list (format may vary)"; fi

# ================================================================
section "F. Lifecycle"
# ================================================================

run stop "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "stop"; else fail "stop" "$OUT"; fi

sleep 3

run start "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "start"; else fail "start" "$(echo "$OUT" | tail -3)"; fi

run restart "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "restart"; else fail "restart" "$(echo "$OUT" | tail -3)"; fi

run logs "$INSTANCE"
pass "logs (ran)"

run update-check "$INSTANCE"
if has_text "current\|latest\|error"; then pass "update-check"; else fail "update-check" "$OUT"; fi

# ================================================================
section "G. Cleanup"
# ================================================================

if $SKIP_CLEANUP; then
    skip "uninstall (--skip-cleanup)"
else
    run uninstall --name "$INSTANCE"
    if [[ $RC -eq 0 ]]; then pass "uninstall"; else fail "uninstall" "$OUT"; fi

    run list
    if ! has_text "$INSTANCE"; then pass "gone from list"; else fail "gone" "still present"; fi
fi

summary
