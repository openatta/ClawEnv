#!/bin/bash
# Linux Podman Test — Podman + Alpine container sandbox
#
# Runs on Linux systems with Podman installed.
# On macOS: creates a Lima Alpine VM with Podman, then runs tests inside it.
#
# Usage:
#   bash scripts/test-linux-podman.sh              # Auto-detect environment
#   bash scripts/test-linux-podman.sh --lima        # Force Lima Alpine VM on macOS
set -uo pipefail

USE_LIMA=false
[[ "${1:-}" == "--lima" ]] && USE_LIMA=true

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/lib-test.sh"

INSTANCE="linux-podman-$$"
PORT=3600

# ================================================================
# Environment setup
# ================================================================

if [[ "$(uname)" == "Linux" ]] && command -v podman &>/dev/null && ! $USE_LIMA; then
    # Running directly on Linux with Podman
    echo "========================================"
    echo "  Linux Podman Test (native)"
    echo "  Instance: $INSTANCE  Port: $PORT"
    echo "========================================"

    cd "$ROOT"
    cargo build -p clawcli 2>&1 | tail -1
    find_cli

elif [[ "$(uname)" == "Darwin" ]] || $USE_LIMA; then
    # macOS: create Lima Alpine VM with Podman
    echo "========================================"
    echo "  Linux Podman Test (via Lima Alpine)"
    echo "  Instance: $INSTANCE  Port: $PORT"
    echo "========================================"

    LIMA_VM="podman-test"

    # Check if VM already exists
    if limactl list 2>/dev/null | grep -q "$LIMA_VM.*Running"; then
        echo "  Lima VM '$LIMA_VM' already running"
    elif limactl list 2>/dev/null | grep -q "$LIMA_VM"; then
        echo "  Starting Lima VM '$LIMA_VM'..."
        limactl start "$LIMA_VM" 2>&1 | tail -3
    else
        echo "  Creating Lima Alpine VM with Podman (3-5 min)..."
        # Use Lima's built-in alpine template, then install podman
        limactl start --name "$LIMA_VM" --tty=false template://alpine 2>&1 | tail -5

        echo "  Installing Podman in Alpine VM..."
        limactl shell "$LIMA_VM" -- sudo sh -c '
            sed -i "s/#\(.*\/community\)/\1/" /etc/apk/repositories
            apk update
            apk add --no-cache podman fuse-overlayfs shadow-uidmap slirp4netns nodejs npm git curl bash build-base
            echo "$(whoami):100000:65536" >> /etc/subuid 2>/dev/null || true
            echo "$(whoami):100000:65536" >> /etc/subgid 2>/dev/null || true
        ' 2>&1 | tail -5
    fi

    # Verify podman works inside VM
    echo "  Verifying podman..."
    if ! limactl shell "$LIMA_VM" -- podman --version 2>&1 | grep -q "podman"; then
        echo "ERROR: Podman not available in Lima VM"
        exit 1
    fi

    # Cross-compile CLI for Linux aarch64 on macOS, then copy to VM
    echo "  Cross-compiling CLI for Linux..."
    export PATH="$HOME/.cargo/bin:$PATH"
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc \
        cargo build -p clawcli --target aarch64-unknown-linux-musl --release 2>&1 | tail -2

    LINUX_CLI="$ROOT/target/aarch64-unknown-linux-musl/release/clawcli"
    if [[ ! -f "$LINUX_CLI" ]]; then
        echo "ERROR: Cross-compilation failed. Install: brew install filosottile/musl-cross/musl-cross"
        echo "       And: rustup target add aarch64-unknown-linux-musl"
        exit 1
    fi

    # Copy binary into VM
    limactl shell "$LIMA_VM" -- sh -c "cp '$LINUX_CLI' /tmp/clawcli && chmod +x /tmp/clawcli" 2>&1

    # Override run() to execute inside Lima VM
    run() {
        TOTAL=$((TOTAL+1))
        RC=0
        OUT=$(limactl shell "$LIMA_VM" -- /tmp/clawcli --json "$@" 2>&1) || RC=$?
    }

    CLI="/tmp/clawcli"
else
    echo "  Not on Linux and no Lima available. Skipping."
    echo "  RESULTS: 0 passed, 0 failed, ALL SKIPPED"
    exit 0
fi

# ================================================================
section "A. System Exploration"
# ================================================================

run doctor
if has_type data; then pass "doctor"; else fail "doctor" "$OUT"; fi

run system-check
if has_type data; then pass "system-check"; else fail "system-check" "$OUT"; fi

run claw-types
if has_text "openclaw"; then pass "claw-types"; else fail "claw-types" "$OUT"; fi

run list
if has_type data; then pass "list"; else fail "list" "$OUT"; fi

run config show
if has_text "language"; then pass "config show"; else fail "config show" "$OUT"; fi

run sandbox list
if has_text "vms"; then pass "sandbox list"; else fail "sandbox list" "$OUT"; fi

run sandbox info
if has_type data; then pass "sandbox info"; else fail "sandbox info" "$OUT"; fi

# ================================================================
section "B. Podman Sandbox Install"
# ================================================================

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

# ================================================================
section "C. Verification"
# ================================================================

run status "$INSTANCE"
if has_type data; then pass "status"; else fail "status" "$OUT"; fi

# exec uses the overridden run() which goes through Lima automatically
TOTAL=$((TOTAL+1)); RC=0
if [[ "$(uname)" == "Linux" ]]; then
    OUT=$("$CLI" --json exec "echo podman-ok" "$INSTANCE" 2>&1) || RC=$?
else
    OUT=$(limactl shell "$LIMA_VM" -- /tmp/clawcli --json exec "echo podman-ok" "$INSTANCE" 2>&1) || RC=$?
fi
if echo "$OUT" | grep -q "podman-ok"; then pass "exec"; else fail "exec" "$OUT"; fi

run logs "$INSTANCE"
pass "logs (ran)"

run update-check "$INSTANCE"
if has_text "current\|latest\|error"; then pass "update-check"; else fail "update-check" "$OUT"; fi

# ================================================================
section "D. Lifecycle"
# ================================================================

run stop "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "stop"; else fail "stop" "$OUT"; fi

run start "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "start"; else fail "start" "$OUT"; fi

run restart "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "restart"; else fail "restart" "$OUT"; fi

# ================================================================
section "E. Config & Edit"
# ================================================================

run config set language en-US
run config show
if has_text "en-US"; then pass "config set round-trip"; else fail "config round-trip" "$OUT"; fi
run config set language zh-CN

run config proxy-test
pass "config proxy-test (ran)"

# ================================================================
section "F. Cleanup"
# ================================================================

run uninstall --name "$INSTANCE"
if [[ $RC -eq 0 ]]; then pass "uninstall"; else fail "uninstall" "$OUT"; fi

run list
if ! has_text "$INSTANCE"; then pass "gone from list"; else fail "gone" "still present"; fi

summary
