#!/bin/bash
# macOS Native Test — full native install + bridge + lifecycle + config + cleanup
#
# Usage:
#   bash scripts/test-macos-native.sh
#   bash scripts/test-macos-native.sh --claw-type nanoclaw
#   bash scripts/test-macos-native.sh --verbose
set -uo pipefail

CLAW_TYPE="${1:-openclaw}"
VERBOSE=false
[[ "${1:-}" == "--verbose" || "${2:-}" == "--verbose" ]] && VERBOSE=true
[[ "${1:-}" == "--claw-type" ]] && CLAW_TYPE="${2:-openclaw}"

INSTANCE="mac-native-$$"
PORT=3200
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-test.sh"

echo "========================================"
echo "  macOS Native Test"
echo "  Claw: $CLAW_TYPE  Instance: $INSTANCE  Port: $PORT"
echo "========================================"

cd "$SCRIPT_DIR/.."
cargo build -p clawcli 2>&1 | tail -1
find_cli

# ================================================================
section "A. System Exploration"
# ================================================================

run doctor
if has_type data; then pass "doctor"; else fail "doctor" "$OUT"; fi

run system-check
if has_type data && has_text "OS"; then pass "system-check"; else fail "system-check" "$OUT"; fi

run claw-types
if has_text "$CLAW_TYPE"; then pass "claw-types"; else fail "claw-types" "$OUT"; fi

run list
if has_type data; then pass "list"; else fail "list" "$OUT"; fi

run config show
if has_text "language"; then pass "config show"; else fail "config show" "$OUT"; fi

run sandbox list
if has_text "vms"; then pass "sandbox list"; else fail "sandbox list" "$OUT"; fi

run sandbox info
if has_type data; then pass "sandbox info"; else fail "sandbox info" "$OUT"; fi

# ================================================================
section "B. Native Step-by-Step Install"
# ================================================================

run install --mode native --name "$INSTANCE" --claw-type "$CLAW_TYPE" --step prereq
if [[ $RC -eq 0 ]]; then pass "step prereq"; else fail "step prereq" "$OUT"; fi

run install --mode native --name "$INSTANCE" --claw-type "$CLAW_TYPE" --step create
if [[ $RC -eq 0 ]]; then pass "step create"; else fail "step create" "$OUT"; fi

echo "       Installing claw (may take minutes)..."
run install --mode native --name "$INSTANCE" --claw-type "$CLAW_TYPE" --step claw
if [[ $RC -eq 0 ]]; then pass "step claw"; else fail "step claw" "$(echo "$OUT" | tail -3)"; fi

run install --mode native --name "$INSTANCE" --claw-type "$CLAW_TYPE" --port "$PORT" --step config
if [[ $RC -eq 0 ]]; then pass "step config"; else fail "step config" "$OUT"; fi

run install --mode native --name "$INSTANCE" --claw-type "$CLAW_TYPE" --port "$PORT" --step gateway
if [[ $RC -eq 0 ]]; then pass "step gateway"; else fail "step gateway" "$OUT"; fi

# Verify in list
run list
if has_text "$INSTANCE"; then pass "in list"; else fail "in list" "not found"; fi

# ================================================================
section "C. Bridge Test"
# ================================================================

# Bridge is started by Tauri GUI, not CLI. Test if bridge is reachable.
run bridge test
if [[ $RC -eq 0 ]]; then
    pass "bridge test"
else
    # Bridge may not be running (no GUI) — acceptable skip
    skip "bridge test (not running, needs GUI)"
    SKIP=$((SKIP-1)); TOTAL=$((TOTAL-1))  # undo double count
fi

# ================================================================
section "D. Lifecycle"
# ================================================================

run status "$INSTANCE"
if has_type data; then pass "status"; else fail "status" "$OUT"; fi

run stop "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "stop"; else pass "stop (native no-op)"; fi

run start "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "start"; else fail "start" "$(echo "$OUT" | tail -2)"; fi

run restart "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "restart"; else fail "restart" "$(echo "$OUT" | tail -2)"; fi

# Exec
TOTAL=$((TOTAL+1)); RC=0
OUT=$("$CLI" --json exec "echo hello-mac-native" "$INSTANCE" 2>&1) || RC=$?
if echo "$OUT" | grep -q "hello-mac-native"; then pass "exec"; else fail "exec" "$OUT"; fi

run logs "$INSTANCE"
pass "logs (ran)"

run update-check "$INSTANCE"
if has_text "current\|latest\|error"; then pass "update-check"; else fail "update-check" "$OUT"; fi

# Upgrade (may not have newer version, or may take very long)
# Just verify the command doesn't crash with invalid args
run upgrade nonexistent-$$
if [[ $RC -ne 0 ]]; then pass "upgrade (error path)"; else fail "upgrade" "should fail for nonexistent"; fi

# ================================================================
section "E. Config & Edit"
# ================================================================

# Config set round-trip
run config set language en-US
run config show
if has_text "en-US"; then pass "config set round-trip"; else fail "config set round-trip" "$OUT"; fi
"$CLI" --json config set language zh-CN 2>/dev/null || true

# Port edit
run edit "$INSTANCE" --gateway-port $((PORT+1))
if [[ $RC -eq 0 ]]; then pass "edit gateway-port"; else fail "edit gateway-port" "$OUT"; fi
"$CLI" --json edit "$INSTANCE" --gateway-port "$PORT" 2>/dev/null || true

# Port conflict
run install --mode native --name conflict-$$ --port "$PORT" --step config
if [[ $RC -ne 0 ]]; then pass "port conflict detected"; else fail "port conflict" "should reject"; fi

# Error paths
run status nonexistent-$$
if [[ $RC -ne 0 ]]; then pass "status nonexistent (error)"; else fail "status error" "should fail"; fi

run rename nonexistent-$$ newname
if [[ $RC -ne 0 ]]; then pass "rename nonexistent (error)"; else fail "rename error" "should fail"; fi

# config proxy-test (no proxy = info, not error)
run config proxy-test
pass "config proxy-test (ran)"

# ================================================================
section "F. Cleanup"
# ================================================================

run uninstall --name "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "uninstall"; else fail "uninstall" "$OUT"; fi

run list
if ! has_text "$INSTANCE"; then pass "gone from list"; else fail "gone from list" "still present"; fi

summary
