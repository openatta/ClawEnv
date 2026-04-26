#!/bin/bash
# Negative smoke probe — install must FAIL FAST when host egress is
# blocked, not silently hang for 30+ minutes.
#
# Contract under test: with a black-hole HTTPS_PROXY set, `clawcli
# install` bails within DEADLINE seconds AND emits a structured
# `{type:"error"}` event before exiting. Catches the entire class of
# "30-minute silent hang" regressions that v0.2.x suffered.
#
# Real-machine observation (2026-04-25): on macOS, limactl itself
# refuses to spawn when its env-inherited HTTPS_PROXY can't reach
# upstream — bail happens at VM-creation stage in ~2s. That's
# acceptable: any layer can fail fast, the contract is "no hang."
# A future test variant pointed at `clawcli download fetch` would
# directly exercise the v2 DownloadOps triple-deadline gate
# (CONNECT_TIMEOUT / CHUNK_STALL / MIN_THROUGHPUT).
#
# Wall budget: 90s (120s deadline + 30s margin for cleanup).

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

case "$(uname -s)" in
    Darwin) : ;;
    *) _skip "macOS-only scenario for now (Linux variant TBD)" ;;
esac

# Use a deliberately black-hole proxy. 127.0.0.1:1 is reserved
# (tcpmux) and never serves an HTTP proxy on a dev machine. If it
# happens to be in use locally, the operator should re-run with their
# own black-hole address via E2E_BLOCKED_PROXY.
BLOCKED="${E2E_BLOCKED_PROXY:-http://127.0.0.1:1}"
echo ">> using black-hole proxy: $BLOCKED" >&2

# Sanity: confirm the black-hole address actually black-holes. If a
# real proxy is somehow listening, this scenario can't measure what it
# claims to measure — skip rather than emit a false PASS.
if curl -sSf -m 3 --proxy "$BLOCKED" --head https://example.com/ >/dev/null 2>&1; then
    _skip "$BLOCKED unexpectedly accepted CONNECT — pick a different black hole via E2E_BLOCKED_PROXY"
fi

export HTTP_PROXY="$BLOCKED"
export HTTPS_PROXY="$BLOCKED"
export ALL_PROXY="$BLOCKED"

NAME="probe-mac-blocked-egress"
PORT="11901"

cli instance destroy "$NAME" 2>/dev/null || true

# Run install in the background so we can cap wall time. clawcli
# should bail well within DEADLINE seconds via the download triple-
# deadline path; if it hangs past DEADLINE, the test fails.
DEADLINE=120
LOG="${E2E_TEST_HOME:-/tmp}/clawenv-blocked-install.log"

echo ">> install with blocked egress (expecting fast fail within ${DEADLINE}s)" >&2
START=$(date +%s)
set +e
"$(e2e_cli_bin)" --json install openclaw --backend lima --version latest \
    --name "$NAME" --port "$PORT" >"$LOG" 2>&1 &
PID=$!

# Poll for exit, kill after DEADLINE.
HUNG=0
while kill -0 "$PID" 2>/dev/null; do
    ELAPSED=$(( $(date +%s) - START ))
    if [ "$ELAPSED" -gt "$DEADLINE" ]; then
        HUNG=1
        echo ">> deadline exceeded (${ELAPSED}s > ${DEADLINE}s) — killing install" >&2
        kill -TERM "$PID" 2>/dev/null || true
        sleep 5
        kill -KILL "$PID" 2>/dev/null || true
        break
    fi
    sleep 2
done
wait "$PID" 2>/dev/null
RC=$?
ELAPSED=$(( $(date +%s) - START ))
set -e

echo ">> install exited rc=$RC after ${ELAPSED}s" >&2

# Cleanup any partial state regardless of outcome.
cli instance destroy "$NAME" 2>/dev/null || true

if [ "$HUNG" = 1 ]; then
    _fail "install hung past ${DEADLINE}s deadline — triple-deadline gate did not fire"
    echo "  log tail:" >&2
    tail -20 "$LOG" >&2
    exit 1
fi

# rc=0 would mean install succeeded against a black hole — impossible
# unless the proxy isn't actually blocked, or we accidentally bypassed
# it. Either way, that's a test failure.
if [ "$RC" = 0 ]; then
    _fail "install returned rc=0 with black-hole proxy — proxy was not honoured"
    exit 1
fi

# Expected path: clawcli emitted a structured Error event before the
# deadline. Confirm the JSON stream actually contains an error event
# (not just a process-level failure with empty output).
if ! grep -q '"type":"error"' "$LOG" 2>/dev/null; then
    _fail "install failed (rc=$RC) but emitted no JSON error event — wire protocol broken under blocked egress"
    echo "  log tail:" >&2
    tail -20 "$LOG" >&2
    exit 1
fi

_ok "install bailed cleanly in ${ELAPSED}s with structured error (rc=$RC)"

# Optionally surface the error message — useful when triaging CI logs.
ERR_MSG=$(grep '"type":"error"' "$LOG" | tail -1 | jq -r '.message // ""' 2>/dev/null)
if [ -n "$ERR_MSG" ]; then
    echo "  surfaced error: ${ERR_MSG:0:200}" >&2
fi
