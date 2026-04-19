#!/bin/bash
# ClawEnv E2E test runner.
# Usage:
#   ./tests/e2e/run.sh all                    # all 4 scenarios (~60 min)
#   ./tests/e2e/run.sh 01-sandbox-noproxy     # one scenario
#   ./tests/e2e/run.sh --skip-proxy all       # skip proxy scenarios
# See docs/24-e2e-testing.md for the full spec.

set -eu

# Repo root = two parents up from this script.
export E2E_REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# ──────────────────────────────────────────────────────────────
# Defaults + flag parsing
# ──────────────────────────────────────────────────────────────
E2E_PROXY_UPSTREAM_HOST="127.0.0.1"
E2E_PROXY_UPSTREAM_PORT="7890"
export E2E_PROXY_LISTEN="10080"
# `--proxy-bind` controls the interface the mini-proxy binds to.
# Default 127.0.0.1 (safe, local-only). Windows scenarios auto-switch
# to 0.0.0.0 so the VM can reach it via the Mac's bridged interface.
E2E_PROXY_BIND="127.0.0.1"
export E2E_BUNDLE_DIR="$HOME/Desktop/ClawEnv"
E2E_KEEP_HOME="0"
E2E_SKIP_PROXY="0"
# Windows scenarios (05+) require SSH to a pre-configured Windows ARM64
# box. "all" skips them by default; pass --with-windows to opt in, or
# run explicit scenario names.
E2E_WITH_WINDOWS="0"
E2E_WINDOWS_ONLY="0"
export E2E_VERBOSE="0"

SELECTION=()

while [ $# -gt 0 ]; do
    case "$1" in
        --proxy-upstream)
            IFS=':' read -r E2E_PROXY_UPSTREAM_HOST E2E_PROXY_UPSTREAM_PORT <<<"$2"
            shift 2 ;;
        --proxy-listen)
            export E2E_PROXY_LISTEN="$2"
            shift 2 ;;
        --proxy-bind)
            E2E_PROXY_BIND="$2"
            shift 2 ;;
        --bundle-dir)
            export E2E_BUNDLE_DIR="$2"
            shift 2 ;;
        --keep-home)
            E2E_KEEP_HOME="1"
            shift ;;
        --skip-proxy)
            E2E_SKIP_PROXY="1"
            shift ;;
        --with-windows)
            E2E_WITH_WINDOWS="1"
            shift ;;
        --windows-only)
            E2E_WINDOWS_ONLY="1"
            E2E_WITH_WINDOWS="1"
            shift ;;
        --verbose)
            export E2E_VERBOSE="1"
            shift ;;
        -h|--help)
            head -20 "$0" | tail -10
            exit 0 ;;
        *)
            SELECTION+=("$1")
            shift ;;
    esac
done

if [ "${#SELECTION[@]}" -eq 0 ]; then
    echo "Usage: $0 [options] all|<scenario-name>" >&2
    echo "Scenarios:" >&2
    ls "$E2E_REPO_ROOT/tests/e2e/scenarios/" | sed 's/\.sh$//' | sed 's/^/  /' >&2
    exit 2
fi

# Expand `all` to scenario set. Windows is opt-in via --with-windows
# because it requires a pre-configured SSH target (.env + keys).
if [ "${SELECTION[0]}" = "all" ]; then
    SELECTION=()
    for f in "$E2E_REPO_ROOT/tests/e2e/scenarios/"*.sh; do
        [ -f "$f" ] || continue
        name=$(basename "$f" .sh)
        is_windows="0"
        [[ "$name" == *windows* ]] && is_windows="1"

        if [ "$E2E_WINDOWS_ONLY" = "1" ] && [ "$is_windows" = "0" ]; then continue; fi
        if [ "$E2E_WITH_WINDOWS" = "0" ] && [ "$is_windows" = "1" ]; then continue; fi
        if [ "$E2E_SKIP_PROXY" = "1" ] && [[ "$name" == *-proxy ]]; then continue; fi
        SELECTION+=("$name")
    done
fi

# ──────────────────────────────────────────────────────────────
# Source libraries
# ──────────────────────────────────────────────────────────────
# shellcheck source=lib/isolate.sh
source "$E2E_REPO_ROOT/tests/e2e/lib/isolate.sh"
# shellcheck source=lib/cli.sh
source "$E2E_REPO_ROOT/tests/e2e/lib/cli.sh"
# shellcheck source=lib/assert.sh
source "$E2E_REPO_ROOT/tests/e2e/lib/assert.sh"
# shellcheck source=lib/proxy-mock.sh
source "$E2E_REPO_ROOT/tests/e2e/lib/proxy-mock.sh"

# ──────────────────────────────────────────────────────────────
# Preflight
# ──────────────────────────────────────────────────────────────
if ! command -v jq >/dev/null; then
    echo "✗ jq not installed. \`brew install jq\`" >&2
    exit 3
fi
if ! command -v python3 >/dev/null; then
    echo "✗ python3 not found" >&2
    exit 3
fi

# Check clawcli is built.
CLI_BIN="$(e2e_cli_bin)"
if [ ! -x "$CLI_BIN" ] && ! command -v "$CLI_BIN" >/dev/null; then
    echo "✗ clawcli not built. Run \`cargo build -p clawcli --release\`." >&2
    exit 3
fi
echo "[runner] clawcli = $CLI_BIN" >&2

# Do any selected scenarios need a proxy?
NEEDS_PROXY="0"
NEEDS_WIN_REACHABLE_PROXY="0"
for name in "${SELECTION[@]}"; do
    if [[ "$name" == *-proxy ]]; then NEEDS_PROXY="1"; fi
    if [[ "$name" == *windows*-proxy ]]; then NEEDS_WIN_REACHABLE_PROXY="1"; fi
done
# Windows scenarios need the proxy bound where the Windows VM can reach
# it. 127.0.0.1 isn't reachable from the VM — override to 0.0.0.0.
if [ "$NEEDS_WIN_REACHABLE_PROXY" = "1" ] && [ "$E2E_PROXY_BIND" = "127.0.0.1" ]; then
    echo "[runner] Windows proxy scenario: auto-binding proxy to 0.0.0.0" >&2
    E2E_PROXY_BIND="0.0.0.0"
fi

# Check upstream proxy availability if needed.
if [ "$NEEDS_PROXY" = "1" ]; then
    if ! e2e_proxy_check_upstream "$E2E_PROXY_UPSTREAM_HOST" "$E2E_PROXY_UPSTREAM_PORT"; then
        echo "⚠ upstream proxy ${E2E_PROXY_UPSTREAM_HOST}:${E2E_PROXY_UPSTREAM_PORT} unreachable." >&2
        echo "   Start your local proxy (Clash/Surge/...) or pass --skip-proxy." >&2
        exit 4
    fi
fi

# ──────────────────────────────────────────────────────────────
# Setup: isolate home + start mini-proxy if needed
# ──────────────────────────────────────────────────────────────
e2e_isolate_setup

cleanup() {
    local rc=$?
    e2e_proxy_stop 2>/dev/null || true
    e2e_isolate_teardown "$E2E_KEEP_HOME"
    exit $rc
}
trap cleanup EXIT INT TERM

if [ "$NEEDS_PROXY" = "1" ]; then
    if ! e2e_proxy_start \
            "$E2E_PROXY_LISTEN" \
            "$E2E_PROXY_UPSTREAM_HOST" \
            "$E2E_PROXY_UPSTREAM_PORT" \
            "$E2E_PROXY_BIND"; then
        echo "✗ mini-proxy failed to start" >&2
        exit 5
    fi
fi

# ──────────────────────────────────────────────────────────────
# Run scenarios serially
# ──────────────────────────────────────────────────────────────
TOTAL="${#SELECTION[@]}"
PASSED=0
FAILED_LIST=()
STARTED_AT=$(date +%s)

for i in "${!SELECTION[@]}"; do
    name="${SELECTION[$i]}"
    script="$E2E_REPO_ROOT/tests/e2e/scenarios/${name}.sh"

    if [ ! -f "$script" ]; then
        echo "✗ scenario not found: $name" >&2
        FAILED_LIST+=("$name (missing)")
        continue
    fi

    echo ""
    echo "════════════════════════════════════════════════════════════"
    echo "▶ [$((i+1))/$TOTAL] $name"
    echo "════════════════════════════════════════════════════════════"
    local_started=$(date +%s)

    # Scenario runs in a subshell so `set -e` + assertion failures don't
    # abort the runner. Use `source`, not `bash` — bash spawns a fresh
    # process that loses all the helpers we sourced. source runs in the
    # subshell's own context so functions are visible.
    if (
        set -eu
        source "$E2E_REPO_ROOT/tests/e2e/lib/isolate.sh"
        source "$E2E_REPO_ROOT/tests/e2e/lib/cli.sh"
        source "$E2E_REPO_ROOT/tests/e2e/lib/assert.sh"
        source "$E2E_REPO_ROOT/tests/e2e/lib/proxy-mock.sh"
        source "$script"
    ); then
        elapsed=$(( $(date +%s) - local_started ))
        echo "✓ $name PASSED (${elapsed}s)"
        PASSED=$((PASSED + 1))
        e2e_archive_log "$name" "pass"
    else
        elapsed=$(( $(date +%s) - local_started ))
        echo "✗ $name FAILED (${elapsed}s)"
        FAILED_LIST+=("$name")
        e2e_archive_log "$name" "fail"
    fi
done

# ──────────────────────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────────────────────
TOTAL_ELAPSED=$(( $(date +%s) - STARTED_AT ))
echo ""
echo "════════════════════════════════════════════════════════════"
echo " Summary — ${PASSED}/${TOTAL} passed — total ${TOTAL_ELAPSED}s"
if [ "${#FAILED_LIST[@]}" -gt 0 ]; then
    echo " FAILED:"
    for f in "${FAILED_LIST[@]}"; do echo "   ✗ $f"; done
fi
echo "════════════════════════════════════════════════════════════"

if [ "${#FAILED_LIST[@]}" -eq 0 ]; then
    exit 0
else
    exit 1
fi
