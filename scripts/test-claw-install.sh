#!/bin/bash
# ClawEnv — Quick claw install test (install + verify only, no gateway)
#
# Usage:
#   bash scripts/test-claw-install.sh <claw_id> [output_dir] [timeout]
#   bash scripts/test-claw-install.sh --all [output_dir]

set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-test-common.sh"

REGISTRY="$SCRIPT_DIR/../assets/claw-registry.toml"

# ---- All mode ----
if [ "${1:-}" = "--all" ]; then
    OUTPUT_DIR="${2:-./test-results}"
    ALL_CLAWS=$(awk '/^id = / { gsub(/"/, "", $3); printf "%s ", $3 }' "$REGISTRY")
    PASS=0; FAIL=0; TOTAL=0
    for CID in $ALL_CLAWS; do
        TOTAL=$((TOTAL + 1))
        echo "=== [$TOTAL] $CID ==="
        if bash "$0" "$CID" "$OUTPUT_DIR"; then PASS=$((PASS + 1)); else FAIL=$((FAIL + 1)); fi
    done
    echo ""; echo "Total: $TOTAL | Pass: $PASS | Fail: $FAIL"
    [ "$FAIL" -eq 0 ] && exit 0 || exit 1
fi

# ---- Single claw mode ----
CLAW_ID="${1:?Usage: test-claw-install.sh <claw_id> [output_dir] [timeout]}"
OUTPUT_DIR="${2:-./test-results}"
TIMEOUT="${3:-900}"
VM_NAME="clawenv-test-${CLAW_ID}"
RESULT_FILE="$OUTPUT_DIR/result-${CLAW_ID}.toml"
PLATFORM=$(detect_platform)

mkdir -p "$OUTPUT_DIR"

NPM_PKG=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "npm_package")
CLI_BIN=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "cli_binary")
VERSION_CMD=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "version_cmd")

if [ -z "$NPM_PKG" ] || [ -z "$CLI_BIN" ]; then
    echo "  SKIP: $CLAW_ID not found in registry"
    cat > "$RESULT_FILE" << EOF
[result]
claw_id = "$CLAW_ID"
status = "skip"
error = "not in registry"
EOF
    exit 0
fi

cleanup() { destroy_test_sandbox "$PLATFORM" "$VM_NAME"; }
trap cleanup EXIT

START=$(now_sec)

# Step 1: Create sandbox
echo "  Creating sandbox..."
if ! create_test_sandbox "$PLATFORM" "$VM_NAME"; then
    echo "  FAIL: sandbox creation"
    cat > "$RESULT_FILE" << EOF
[result]
claw_id = "$CLAW_ID"
status = "fail"
error = "sandbox creation failed"
EOF
    exit 1
fi

# Step 2: Install with timeout
echo "  Installing ${NPM_PKG}@latest (timeout: ${TIMEOUT}s)..."
INSTALL_START=$(now_sec)

run_with_timeout "$TIMEOUT" \
    sandbox_exec "$PLATFORM" "$VM_NAME" "sudo npm install -g ${NPM_PKG}@latest 2>&1"
INSTALL_RC=$?
INSTALL_DUR=$(( $(now_sec) - INSTALL_START ))

TIMED_OUT=false
[ "$INSTALL_RC" -eq 124 ] && TIMED_OUT=true

# Step 3: Verify
echo "  Verifying..."
VER=$(sandbox_exec "$PLATFORM" "$VM_NAME" "which $CLI_BIN && $CLI_BIN $VERSION_CMD" 2>/dev/null) || VER=""
VER_LINE=$(echo "$VER" | tail -1 | tr -d '\r"')

PKG_SIZE=$(sandbox_exec "$PLATFORM" "$VM_NAME" \
    "du -sh /usr/lib/node_modules/${NPM_PKG} 2>/dev/null | awk '{print \$1}'" 2>/dev/null) || PKG_SIZE="?"

TOTAL_DUR=$(( $(now_sec) - START ))

# Determine status
if [ "$TIMED_OUT" = true ]; then
    STATUS="timeout"
elif [ -n "$VER_LINE" ] && ! echo "$VER_LINE" | grep -qi "not found"; then
    STATUS="pass"
else
    STATUS="fail"
fi

echo "  Result: $STATUS ($VER_LINE) install=${INSTALL_DUR}s total=${TOTAL_DUR}s"

cat > "$RESULT_FILE" << EOF
[result]
claw_id = "$CLAW_ID"
npm_package = "$NPM_PKG"
cli_binary = "$CLI_BIN"
platform = "$PLATFORM"
status = "$STATUS"
version = "$VER_LINE"
package_size = "$PKG_SIZE"
install_duration_sec = $INSTALL_DUR
total_duration_sec = $TOTAL_DUR
timestamp = "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

[[steps]]
name = "install"
status = "$([ "$INSTALL_RC" -eq 0 ] && echo "pass" || echo "fail")"
duration_sec = $INSTALL_DUR

[[steps]]
name = "verify_version"
status = "$([ -n "$VER_LINE" ] && echo "pass" || echo "fail")"
duration_sec = 0
detail = "$VER_LINE"
EOF

[ "$STATUS" = "pass" ] && exit 0 || exit 1
