#!/bin/bash
# Linux Podman Test — Podman + Alpine container (DEFERRED)
#
# This test requires a Linux environment with Podman installed.
# Can be run on a Linux machine, CI container, or Lima Ubuntu VM on macOS.
#
# When ready to implement:
#   1. Ensure Podman available: clawenv install --mode sandbox --step prereq
#   2. Build container: clawenv install --mode sandbox --step create
#   3. Install claw: clawenv install --mode sandbox --step claw
#   4. Full lifecycle: start/stop/restart/exec/logs
set -uo pipefail

echo "========================================"
echo "  Linux Podman Test"
echo "  STATUS: DEFERRED — requires Linux environment with Podman"
echo "========================================"
echo ""

# Check if we're actually on Linux
if [[ "$(uname)" == "Linux" ]] && command -v podman &>/dev/null; then
    echo "  Podman detected! Running tests..."
    echo ""

    INSTANCE="linux-podman-$$"
    PORT=3400
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    source "$SCRIPT_DIR/lib-test.sh"

    cd "$SCRIPT_DIR/.."
    cargo build -p clawenv-cli 2>&1 | tail -1
    find_cli

    section "A. System Exploration"
    run doctor
    if has_type data; then pass "doctor"; else fail "doctor" "$OUT"; fi

    run system-check
    if has_type data; then pass "system-check"; else fail "system-check" "$OUT"; fi

    run sandbox list
    if has_text vms; then pass "sandbox list"; else fail "sandbox list" "$OUT"; fi

    section "B. Podman Sandbox Install"
    echo "       Creating Podman container..."
    run install --mode sandbox --name "$INSTANCE" --step prereq
    if [[ $RC -eq 0 ]]; then pass "prereq"; else fail "prereq" "$OUT"; fi

    run install --mode sandbox --name "$INSTANCE" --step create
    if [[ $RC -eq 0 ]]; then pass "create"; else fail "create" "$(echo "$OUT" | tail -5)"; fi

    echo "       Installing claw..."
    run install --mode sandbox --name "$INSTANCE" --step claw
    if [[ $RC -eq 0 ]]; then pass "claw install"; else fail "claw" "$(echo "$OUT" | tail -3)"; fi

    run install --mode sandbox --name "$INSTANCE" --port "$PORT" --step config
    if [[ $RC -eq 0 ]]; then pass "config"; else fail "config" "$OUT"; fi

    run install --mode sandbox --name "$INSTANCE" --port "$PORT" --step gateway
    if [[ $RC -eq 0 ]]; then pass "gateway"; else fail "gateway" "$OUT"; fi

    section "C. Lifecycle"
    run status "$INSTANCE"
    if has_type data; then pass "status"; else fail "status" "$OUT"; fi

    TOTAL=$((TOTAL+1)); RC=0
    OUT=$("$CLI" --json exec "echo podman-ok" "$INSTANCE" 2>&1) || RC=$?
    if echo "$OUT" | grep -q "podman-ok"; then pass "exec"; else fail "exec" "$OUT"; fi

    run stop "$INSTANCE"
    if [[ $RC -eq 0 ]]; then pass "stop"; else fail "stop" "$OUT"; fi

    run start "$INSTANCE"
    if [[ $RC -eq 0 ]]; then pass "start"; else fail "start" "$OUT"; fi

    section "D. Cleanup"
    run uninstall --name "$INSTANCE"
    if [[ $RC -eq 0 ]]; then pass "uninstall"; else fail "uninstall" "$OUT"; fi

    summary
else
    echo "  Not on Linux or Podman not installed. Skipping."
    echo ""
    echo "  To run on Linux:"
    echo "    sudo apt install podman  # or dnf/pacman"
    echo "    bash scripts/test-linux-podman.sh"
    echo ""
    echo "  RESULTS: 0 passed, 0 failed, ALL SKIPPED (not Linux)"
    exit 0
fi
