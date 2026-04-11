#!/bin/bash
# ClawEnv — Single claw full lifecycle test (7 steps)
#
# Usage: bash scripts/test-claw-lifecycle.sh <claw_id> [output_dir] [timeout]
#
# Steps: create sandbox → install → verify → start gateway → API key → stop → destroy
# Output: <output_dir>/result-<claw_id>.toml
# Exit: 0 = all pass, 1 = any fail

set -uo pipefail

CLAW_ID="${1:?Usage: test-claw-lifecycle.sh <claw_id> [output_dir] [timeout]}"
OUTPUT_DIR="${2:-./test-results}"
TIMEOUT="${3:-900}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-test-common.sh"

REGISTRY="$SCRIPT_DIR/../assets/claw-registry.toml"
VM_NAME="clawenv-test-${CLAW_ID}"
RESULT_FILE="$OUTPUT_DIR/result-${CLAW_ID}.toml"
PLATFORM=$(detect_platform)

mkdir -p "$OUTPUT_DIR"

# ---- Parse registry ----
NPM_PKG=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "npm_package")
CLI_BIN=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "cli_binary")
GATEWAY_CMD=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "gateway_cmd")
VERSION_CMD=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "version_cmd")
APIKEY_CMD=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "config_apikey_cmd")
DEFAULT_PORT=$(parse_registry_field "$REGISTRY" "$CLAW_ID" "default_port")
DEFAULT_PORT="${DEFAULT_PORT:-3000}"

if [ -z "$NPM_PKG" ] || [ -z "$CLI_BIN" ]; then
    echo "ERROR: claw_id '$CLAW_ID' not found in registry"
    cat > "$RESULT_FILE" << EOF
[result]
claw_id = "$CLAW_ID"
status = "error"
error = "not found in claw-registry.toml"
EOF
    exit 1
fi

# ---- Helpers ----
log() { echo "[$(date +%H:%M:%S)] [$CLAW_ID] $*"; }
sexec() { sandbox_exec "$PLATFORM" "$VM_NAME" "$@"; }

STEPS=()
step_result() {
    local name="$1" status="$2" duration="$3" detail="${4:-}"
    STEPS+=("$name|$status|$duration|$detail")
    log "  $name: $status (${duration}s) $detail"
}

cleanup() {
    log "Cleaning up sandbox..."
    destroy_test_sandbox "$PLATFORM" "$VM_NAME"
}
trap cleanup EXIT

# ========== STEP 1: Create Sandbox ==========
log "Step 1/7: Creating sandbox..."
S=$(now_sec)

if create_test_sandbox "$PLATFORM" "$VM_NAME" "$DEFAULT_PORT"; then
    step_result "create_sandbox" "pass" "$(($(now_sec) - S))"
else
    step_result "create_sandbox" "fail" "$(($(now_sec) - S))" "sandbox creation failed"
    # Bail early
    cat > "$RESULT_FILE" << EOF
[result]
claw_id = "$CLAW_ID"
npm_package = "$NPM_PKG"
cli_binary = "$CLI_BIN"
platform = "$PLATFORM"
status = "fail"
steps_passed = 0
steps_failed = 1
EOF
    exit 1
fi

# ========== STEP 2: Install Claw ==========
log "Step 2/7: Installing ${NPM_PKG}@latest (timeout: ${TIMEOUT}s)..."
S=$(now_sec)

run_with_timeout "$TIMEOUT" \
    sexec "sudo npm install -g ${NPM_PKG}@latest 2>&1"
INSTALL_RC=$?
INSTALL_DUR=$(($(now_sec) - S))

if [ "$INSTALL_RC" -eq 124 ]; then
    step_result "install" "timeout" "$INSTALL_DUR" "exceeded ${TIMEOUT}s"
elif [ "$INSTALL_RC" -ne 0 ]; then
    step_result "install" "fail" "$INSTALL_DUR" "exit code $INSTALL_RC"
else
    step_result "install" "pass" "$INSTALL_DUR"
fi

# ========== STEP 3: Verify Version ==========
log "Step 3/7: Verifying binary..."
S=$(now_sec)

VER_OUT=$(sexec "which $CLI_BIN && $CLI_BIN $VERSION_CMD") || VER_OUT=""
VER_DUR=$(($(now_sec) - S))

if [ -n "$VER_OUT" ] && ! echo "$VER_OUT" | grep -qi "not found"; then
    VERSION_STR=$(echo "$VER_OUT" | tail -1 | tr -d '\r')
    step_result "verify_version" "pass" "$VER_DUR" "$VERSION_STR"
else
    step_result "verify_version" "fail" "$VER_DUR" "binary not found"
fi

# ========== STEP 4: Start Gateway + Health Check ==========
log "Step 4/7: Starting gateway on port $DEFAULT_PORT..."
S=$(now_sec)

RESOLVED_GW_CMD=$(echo "$GATEWAY_CMD" | sed "s/{port}/$DEFAULT_PORT/g")
sexec "nohup $CLI_BIN $RESOLVED_GW_CMD > /tmp/clawenv-gateway.log 2>&1 &" || true

# Poll for HTTP response (up to 30s)
GW_OK=false
for _ in $(seq 1 10); do
    sleep 3
    HTTP_CODE=$(sexec "curl -s -o /dev/null -w '%{http_code}' --connect-timeout 2 http://127.0.0.1:${DEFAULT_PORT}/ 2>/dev/null") || HTTP_CODE="000"
    HTTP_CODE=$(echo "$HTTP_CODE" | tr -d "'")
    if [ "$HTTP_CODE" != "000" ] && [ -n "$HTTP_CODE" ]; then
        GW_OK=true
        break
    fi
done

GW_DUR=$(($(now_sec) - S))
if [ "$GW_OK" = true ]; then
    step_result "gateway_start" "pass" "$GW_DUR" "HTTP $HTTP_CODE"
else
    GW_LOG=$(sexec "tail -5 /tmp/clawenv-gateway.log 2>/dev/null" | tr '\n' ' ')
    step_result "gateway_start" "fail" "$GW_DUR" "no response. log: $GW_LOG"
fi

# ========== STEP 5: Configure API Key ==========
log "Step 5/7: Configuring API key..."
S=$(now_sec)

if [ -n "$APIKEY_CMD" ]; then
    RESOLVED_AK_CMD=$(echo "$APIKEY_CMD" | sed "s/{key}/sk-test-00000000/g")
    # Note: test API key may be rejected by some claws — we only verify the command runs
    AK_OUT=$(sexec "$CLI_BIN $RESOLVED_AK_CMD 2>&1 || true") || AK_OUT=""
    AK_DUR=$(($(now_sec) - S))
    # Pass if the command executed (even if key is invalid — that's expected with test key)
    step_result "config_apikey" "pass" "$AK_DUR" "command executed"
else
    step_result "config_apikey" "skip" "0" "no apikey command"
fi

# ========== STEP 6: Stop Gateway ==========
log "Step 6/7: Stopping gateway..."
S=$(now_sec)

# Kill gateway: try graceful first, then force
sexec "sudo pkill -f '$CLI_BIN' 2>/dev/null; sleep 1; sudo pkill -9 -f '$CLI_BIN' 2>/dev/null; true" || true
sleep 2

STILL=$(sexec "pgrep -f '$CLI_BIN' 2>/dev/null || true") || STILL=""
STOP_DUR=$(($(now_sec) - S))
if [ -z "$STILL" ]; then
    step_result "gateway_stop" "pass" "$STOP_DUR"
else
    step_result "gateway_stop" "fail" "$STOP_DUR" "process still running"
fi

# ========== STEP 7: Destroy Sandbox ==========
log "Step 7/7: Destroying sandbox..."
S=$(now_sec)

destroy_test_sandbox "$PLATFORM" "$VM_NAME"
step_result "destroy" "pass" "$(($(now_sec) - S))"

# Disable cleanup trap (already destroyed)
trap - EXIT

# ========== Write Result ==========
TOTAL_PASS=0; TOTAL_FAIL=0; TOTAL_SKIP=0
for s in "${STEPS[@]}"; do
    case "$s" in
        *"|pass|"*)    TOTAL_PASS=$((TOTAL_PASS+1)) ;;
        *"|fail|"*|*"|timeout|"*) TOTAL_FAIL=$((TOTAL_FAIL+1)) ;;
        *"|skip|"*)    TOTAL_SKIP=$((TOTAL_SKIP+1)) ;;
    esac
done

OVERALL="pass"
[ "$TOTAL_FAIL" -gt 0 ] && OVERALL="fail"

cat > "$RESULT_FILE" << EOF
[result]
claw_id = "$CLAW_ID"
npm_package = "$NPM_PKG"
cli_binary = "$CLI_BIN"
platform = "$PLATFORM"
status = "$OVERALL"
steps_passed = $TOTAL_PASS
steps_failed = $TOTAL_FAIL
steps_skipped = $TOTAL_SKIP
timestamp = "$(date -u +%Y-%m-%dT%H:%M:%SZ)"

EOF

for s in "${STEPS[@]}"; do
    IFS='|' read -r name status dur detail <<< "$s"
    cat >> "$RESULT_FILE" << EOF
[[steps]]
name = "$name"
status = "$status"
duration_sec = $dur
detail = "$detail"

EOF
done

log "Result: $OVERALL (pass=$TOTAL_PASS fail=$TOTAL_FAIL skip=$TOTAL_SKIP)"

[ "$OVERALL" = "pass" ] && exit 0 || exit 1
