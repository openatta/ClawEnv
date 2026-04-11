#!/bin/bash
# ClawEnv — Automated test runner for all claw products
#
# Usage:
#   bash scripts/test-claw-runner.sh [options]
#
# Options:
#   --claws "id1 id2 ..."   Test specific claws (default: all from registry)
#   --parallel N             Max parallel tests (default: 2)
#   --timeout N              Timeout per claw in seconds (default: 300)
#   --retry N                Retry failed tests N times (default: 1)
#   --output DIR             Output directory (default: ./test-results)
#   --quick                  Only test install + version (skip gateway/apikey)
#
# Examples:
#   bash scripts/test-claw-runner.sh                             # full test, all claws
#   bash scripts/test-claw-runner.sh --claws "openclaw zeroclaw" # specific claws
#   bash scripts/test-claw-runner.sh --parallel 4 --timeout 600  # fast machine
#   bash scripts/test-claw-runner.sh --quick                     # install-only test
#
# Output:
#   test-results/result-{claw_id}.toml     — per-claw results
#   test-results/summary-{timestamp}.toml  — aggregated summary
#
# Exit: 0 if all pass, 1 if any fail

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-test-common.sh"
REGISTRY="$SCRIPT_DIR/../assets/claw-registry.toml"

# ---- Defaults ----
PARALLEL=2
TIMEOUT=300
RETRY=1
OUTPUT_DIR="./test-results"
QUICK=false
CLAW_LIST=""
TIMESTAMP=$(date +%Y%m%d-%H%M%S)

# ---- Parse args ----
while [[ $# -gt 0 ]]; do
    case "$1" in
        --claws)    CLAW_LIST="$2"; shift 2 ;;
        --parallel) PARALLEL="$2"; shift 2 ;;
        --timeout)  TIMEOUT="$2"; shift 2 ;;
        --retry)    RETRY="$2"; shift 2 ;;
        --output)   OUTPUT_DIR="$2"; shift 2 ;;
        --quick)    QUICK=true; shift ;;
        --help|-h)
            head -20 "$0" | grep '^#' | sed 's/^# *//'
            exit 0 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ---- Auto-detect claws from registry ----
if [ -z "$CLAW_LIST" ]; then
    CLAW_LIST=$(awk '/^id = / { gsub(/"/, "", $3); printf "%s ", $3 }' "$REGISTRY")
fi
CLAW_LIST=$(echo "$CLAW_LIST" | xargs)  # trim

CLAW_COUNT=$(echo "$CLAW_LIST" | wc -w | xargs)

mkdir -p "$OUTPUT_DIR"

# ---- Banner ----
echo "╔══════════════════════════════════════════════════════════╗"
echo "║          ClawEnv — Automated Claw Test Runner           ║"
echo "╠══════════════════════════════════════════════════════════╣"
echo "║  Claws:      $CLAW_COUNT"
echo "║  Parallel:   $PARALLEL"
echo "║  Timeout:    ${TIMEOUT}s"
echo "║  Retry:      $RETRY"
echo "║  Mode:       $([ "$QUICK" = true ] && echo "quick (install only)" || echo "full lifecycle")"
echo "║  Output:     $OUTPUT_DIR"
echo "╚══════════════════════════════════════════════════════════╝"
echo ""

# ---- Choose test script ----
if [ "$QUICK" = true ]; then
    TEST_SCRIPT="$SCRIPT_DIR/test-claw-install.sh"
else
    TEST_SCRIPT="$SCRIPT_DIR/test-claw-lifecycle.sh"
fi

if [ ! -x "$TEST_SCRIPT" ]; then
    echo "ERROR: Test script not found or not executable: $TEST_SCRIPT"
    exit 1
fi

# ---- Run tests with parallel control ----
PIDS=()
CLAW_FOR_PID=()
RUNNING=0

wait_for_slot() {
    while [ "$RUNNING" -ge "$PARALLEL" ]; do
        # Wait for any child to finish
        for i in "${!PIDS[@]}"; do
            if ! kill -0 "${PIDS[$i]}" 2>/dev/null; then
                wait "${PIDS[$i]}" 2>/dev/null || true
                unset "PIDS[$i]"
                unset "CLAW_FOR_PID[$i]"
                RUNNING=$((RUNNING - 1))
                break
            fi
        done
        # Re-pack arrays
        PIDS=("${PIDS[@]}")
        CLAW_FOR_PID=("${CLAW_FOR_PID[@]}")
        [ "$RUNNING" -ge "$PARALLEL" ] && sleep 1
    done
}

echo "Starting tests..."
echo ""

for CLAW_ID in $CLAW_LIST; do
    wait_for_slot

    echo "[START] $CLAW_ID"

    if [ "$QUICK" = true ]; then
        # Quick mode: reuse test-claw-install.sh (single claw)
        bash "$TEST_SCRIPT" "$CLAW_ID" "$OUTPUT_DIR" > "$OUTPUT_DIR/log-${CLAW_ID}.txt" 2>&1 &
    else
        bash "$TEST_SCRIPT" "$CLAW_ID" "$OUTPUT_DIR" "$TIMEOUT" > "$OUTPUT_DIR/log-${CLAW_ID}.txt" 2>&1 &
    fi

    PID=$!
    PIDS+=("$PID")
    CLAW_FOR_PID+=("$CLAW_ID")
    RUNNING=$((RUNNING + 1))
done

# Wait for all remaining
for pid in "${PIDS[@]}"; do
    wait "$pid" 2>/dev/null || true
done

echo ""
echo "All tests completed. Checking results..."
echo ""

# ---- Collect results + retry failures ----
PASS_LIST=""
FAIL_LIST=""
SKIP_LIST=""

for CLAW_ID in $CLAW_LIST; do
    RESULT="$OUTPUT_DIR/result-${CLAW_ID}.toml"
    if [ ! -f "$RESULT" ]; then
        FAIL_LIST="$FAIL_LIST $CLAW_ID"
        continue
    fi

    STATUS=$(awk '/^status = / { gsub(/"/, "", $3); print $3; exit }' "$RESULT")
    case "$STATUS" in
        pass)   PASS_LIST="$PASS_LIST $CLAW_ID" ;;
        skip*)  SKIP_LIST="$SKIP_LIST $CLAW_ID" ;;
        *)      FAIL_LIST="$FAIL_LIST $CLAW_ID" ;;
    esac
done

# Retry failures
RETRY_ROUND=0
while [ "$RETRY_ROUND" -lt "$RETRY" ] && [ -n "$(echo "$FAIL_LIST" | xargs)" ]; do
    RETRY_ROUND=$((RETRY_ROUND + 1))
    RETRY_CLAWS=$(echo "$FAIL_LIST" | xargs)
    echo ""
    echo "=== Retry round $RETRY_ROUND: $RETRY_CLAWS ==="
    echo ""

    FAIL_LIST=""
    RUNNING=0
    PIDS=()

    for CLAW_ID in $RETRY_CLAWS; do
        wait_for_slot

        echo "[RETRY] $CLAW_ID"

        if [ "$QUICK" = true ]; then
            bash "$TEST_SCRIPT" "$CLAW_ID" "$OUTPUT_DIR" > "$OUTPUT_DIR/log-${CLAW_ID}-retry${RETRY_ROUND}.txt" 2>&1 &
        else
            bash "$TEST_SCRIPT" "$CLAW_ID" "$OUTPUT_DIR" "$TIMEOUT" > "$OUTPUT_DIR/log-${CLAW_ID}-retry${RETRY_ROUND}.txt" 2>&1 &
        fi

        PID=$!
        PIDS+=("$PID")
        RUNNING=$((RUNNING + 1))
    done

    for pid in "${PIDS[@]}"; do
        wait "$pid" 2>/dev/null || true
    done

    # Re-check results
    for CLAW_ID in $RETRY_CLAWS; do
        RESULT="$OUTPUT_DIR/result-${CLAW_ID}.toml"
        if [ ! -f "$RESULT" ]; then
            FAIL_LIST="$FAIL_LIST $CLAW_ID"
            continue
        fi
        STATUS=$(awk '/^status = / { gsub(/"/, "", $3); print $3; exit }' "$RESULT")
        case "$STATUS" in
            pass) PASS_LIST="$PASS_LIST $CLAW_ID" ;;
            *)    FAIL_LIST="$FAIL_LIST $CLAW_ID" ;;
        esac
    done
done

# ---- Generate summary report ----
PASS_COUNT=$(echo "$PASS_LIST" | wc -w | xargs)
FAIL_COUNT=$(echo "$FAIL_LIST" | wc -w | xargs)
SKIP_COUNT=$(echo "$SKIP_LIST" | wc -w | xargs)

SUMMARY="$OUTPUT_DIR/summary-${TIMESTAMP}.toml"

cat > "$SUMMARY" << EOF
# ClawEnv Test Summary
# Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)

[meta]
platform = "$(uname -s | tr '[:upper:]' '[:lower:]')"
arch = "$(uname -m)"
timestamp = "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
parallel = $PARALLEL
timeout_sec = $TIMEOUT
retry = $RETRY
mode = "$([ "$QUICK" = true ] && echo "quick" || echo "lifecycle")"

[summary]
total = $CLAW_COUNT
passed = $PASS_COUNT
failed = $FAIL_COUNT
skipped = $SKIP_COUNT
pass_rate = "$(echo "scale=1; $PASS_COUNT * 100 / $CLAW_COUNT" | bc 2>/dev/null || echo "N/A")%"

[passed]
claws = [$(echo "$PASS_LIST" | xargs | sed 's/ /", "/g; s/^/"/; s/$/"/')]

[failed]
claws = [$(echo "$FAIL_LIST" | xargs | sed 's/ /", "/g; s/^/"/; s/$/"/' 2>/dev/null || echo '')]

[skipped]
claws = [$(echo "$SKIP_LIST" | xargs | sed 's/ /", "/g; s/^/"/; s/$/"/' 2>/dev/null || echo '')]
EOF

# Append per-claw timing
echo "" >> "$SUMMARY"
echo "# Per-claw results" >> "$SUMMARY"
for CLAW_ID in $CLAW_LIST; do
    RESULT="$OUTPUT_DIR/result-${CLAW_ID}.toml"
    if [ -f "$RESULT" ]; then
        STATUS=$(awk '/^status = / { gsub(/"/, "", $3); print $3; exit }' "$RESULT")
        # Sum step durations
        TOTAL_DUR=$(awk '/^duration_sec = / { sum += $3 } END { print sum+0 }' "$RESULT")
        echo "# $CLAW_ID: $STATUS (${TOTAL_DUR}s)" >> "$SUMMARY"
    fi
done

# ---- Print summary table ----
echo ""
echo "╔══════════════════════════════════════════════════════════╗"
echo "║                    Test Results                         ║"
echo "╠══════════════════════════════════════════════════════════╣"
echo ""
printf "  %-18s %-10s %s\n" "CLAW" "STATUS" "DURATION"
printf "  %-18s %-10s %s\n" "──────────────────" "──────────" "────────"

for CLAW_ID in $CLAW_LIST; do
    RESULT="$OUTPUT_DIR/result-${CLAW_ID}.toml"
    if [ -f "$RESULT" ]; then
        STATUS=$(awk '/^status = / { gsub(/"/, "", $3); print $3; exit }' "$RESULT")
        TOTAL_DUR=$(awk '/^duration_sec = / { sum += $3 } END { print sum+0 }' "$RESULT")
        case "$STATUS" in
            pass) ICON="✓" ;;
            fail) ICON="✗" ;;
            *)    ICON="○" ;;
        esac
        printf "  %-18s %s %-8s %4ds\n" "$CLAW_ID" "$ICON" "$STATUS" "$TOTAL_DUR"
    else
        printf "  %-18s ○ %-8s %4ds\n" "$CLAW_ID" "no-result" "0"
    fi
done

echo ""
echo "  Total: $CLAW_COUNT | Pass: $PASS_COUNT | Fail: $FAIL_COUNT | Skip: $SKIP_COUNT"
echo ""
echo "  Summary: $SUMMARY"
echo ""
echo "╚══════════════════════════════════════════════════════════╝"

# ---- Exit code ----
[ "$FAIL_COUNT" -eq 0 ] && exit 0 || exit 1
