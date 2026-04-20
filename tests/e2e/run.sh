#!/bin/bash
# ClawEnv E2E test runner.
# Usage:
#   ./tests/e2e/run.sh all
#   ./tests/e2e/run.sh smoke-mac-sandbox-noproxy
#   ./tests/e2e/run.sh --skip-proxy all
#   ./tests/e2e/run.sh --home-suffix 01 <scenario>    # parallel-safe
# See docs/25-smoke-testing.md for the smoke-test matrix.

set -eu

# Repo root = two parents up from this script.
export E2E_REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

# Defaults.
# Each run writes exports under ~/Desktop/clawenv/exports/ — the Desktop
# stays readable to the user and Finder indexes exports for easy manual
# verification. Override with --bundle-dir to put elsewhere.
export E2E_BUNDLE_DIR="${E2E_BUNDLE_DIR:-$HOME/Desktop/clawenv/exports}"
E2E_KEEP_HOME="0"
E2E_SKIP_PROXY="0"
# Windows scenarios require SSH to a pre-configured Windows box
# (tests/e2e/.env at repo root). `all` skips them by default.
E2E_WITH_WINDOWS="0"
E2E_WINDOWS_ONLY="0"
# Parallel-safe HOME suffix. When set, isolate.sh uses
# /tmp/clawenv-e2e-<suffix> instead of a timestamp, so multiple runs in
# parallel don't collide.
export E2E_HOME_SUFFIX="${E2E_HOME_SUFFIX:-}"
export E2E_VERBOSE="0"

SELECTION=()

while [ $# -gt 0 ]; do
    case "$1" in
        --bundle-dir)
            export E2E_BUNDLE_DIR="$2"
            shift 2 ;;
        --home-suffix)
            export E2E_HOME_SUFFIX="$2"
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
            head -9 "$0" | tail -7
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

# Expand `all`. Windows is opt-in via --with-windows.
if [ "${SELECTION[0]}" = "all" ]; then
    SELECTION=()
    for f in "$E2E_REPO_ROOT/tests/e2e/scenarios/"*.sh; do
        [ -f "$f" ] || continue
        name=$(basename "$f" .sh)
        is_windows="0"
        [[ "$name" == *windows* ]] && is_windows="1"
        [[ "$name" == *-win-* ]] && is_windows="1"

        if [ "$E2E_WINDOWS_ONLY" = "1" ] && [ "$is_windows" = "0" ]; then continue; fi
        if [ "$E2E_WITH_WINDOWS" = "0" ] && [ "$is_windows" = "1" ]; then continue; fi
        if [ "$E2E_SKIP_PROXY" = "1" ] && [[ "$name" == *-proxy ]]; then continue; fi
        SELECTION+=("$name")
    done
fi

# ──────────────────────────────────────────────────────────────
# Source libraries
# ──────────────────────────────────────────────────────────────
source "$E2E_REPO_ROOT/tests/e2e/lib/isolate.sh"
source "$E2E_REPO_ROOT/tests/e2e/lib/cli.sh"
source "$E2E_REPO_ROOT/tests/e2e/lib/assert.sh"
source "$E2E_REPO_ROOT/tests/e2e/lib/detect-proxy.sh"
source "$E2E_REPO_ROOT/tests/e2e/lib/prewarm.sh"

# ──────────────────────────────────────────────────────────────
# Preflight
# ──────────────────────────────────────────────────────────────
if ! command -v jq >/dev/null; then
    echo "✗ jq not installed. \`brew install jq\`" >&2
    exit 3
fi

# Detect host proxies once at startup. Scenarios read the E2E_*_PROXY
# exports and skip if their required protocol isn't present.
detect_mac_http_proxy
detect_mac_socks_proxy
if [ "$E2E_WITH_WINDOWS" = "1" ] || [ "$E2E_WINDOWS_ONLY" = "1" ]; then
    if [ -f "$E2E_REPO_ROOT/.env" ]; then
        export $(grep -v '^#' "$E2E_REPO_ROOT/.env" | grep -E '^(WIN_HOST|WIN_USER|WIN_PROJECT)=' | xargs)
    fi
    detect_win_http_proxy
fi
detect_proxy_summary

CLI_BIN="$(e2e_cli_bin)"
if [ ! -x "$CLI_BIN" ] && ! command -v "$CLI_BIN" >/dev/null; then
    echo "✗ clawcli not built. Run \`cargo build -p clawcli --release\`." >&2
    exit 3
fi
echo "[runner] clawcli = $CLI_BIN" >&2
echo "[runner] exports  = $E2E_BUNDLE_DIR" >&2
mkdir -p "$E2E_BUNDLE_DIR" 2>/dev/null || true

# ──────────────────────────────────────────────────────────────
# Setup: isolate home
# ──────────────────────────────────────────────────────────────
e2e_isolate_setup
# Seed isolated HOME with cached lima/git/node from real ~/.clawenv. Each
# concurrent run gets its own HOME (E2E_HOME_SUFFIX) so the copies don't
# collide. See lib/prewarm.sh for what gets copied and why.
e2e_prewarm_seed_home

cleanup() {
    local rc=$?
    e2e_isolate_teardown "$E2E_KEEP_HOME"
    exit $rc
}
trap cleanup EXIT INT TERM

# ──────────────────────────────────────────────────────────────
# Run scenarios serially
# ──────────────────────────────────────────────────────────────
TOTAL="${#SELECTION[@]}"
PASSED=0
FAILED_LIST=()
SKIPPED_LIST=()
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

    # `SKIP` exit code = 77 (GNU autotools convention). Scenarios use
    # `exit 77` when a precondition isn't met (e.g. SOCKS not configured
    # on host). That's PASSED-equivalent for summary purposes.
    rc=0
    (
        set -eu
        source "$E2E_REPO_ROOT/tests/e2e/lib/isolate.sh"
        source "$E2E_REPO_ROOT/tests/e2e/lib/cli.sh"
        source "$E2E_REPO_ROOT/tests/e2e/lib/assert.sh"
        source "$E2E_REPO_ROOT/tests/e2e/lib/detect-proxy.sh"
        source "$E2E_REPO_ROOT/tests/e2e/lib/prewarm.sh"
        source "$script"
    ) || rc=$?

    elapsed=$(( $(date +%s) - local_started ))
    case $rc in
        0)
            echo "✓ $name PASSED (${elapsed}s)"
            PASSED=$((PASSED + 1))
            e2e_archive_log "$name" "pass" ;;
        77)
            echo "↷ $name SKIPPED (${elapsed}s — precondition not met)"
            SKIPPED_LIST+=("$name")
            e2e_archive_log "$name" "skip" ;;
        *)
            echo "✗ $name FAILED (${elapsed}s — rc=$rc)"
            FAILED_LIST+=("$name")
            e2e_archive_log "$name" "fail" ;;
    esac
done

# ──────────────────────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────────────────────
TOTAL_ELAPSED=$(( $(date +%s) - STARTED_AT ))
echo ""
echo "════════════════════════════════════════════════════════════"
echo " Summary — ${PASSED}/${TOTAL} passed — total ${TOTAL_ELAPSED}s"
if [ "${#SKIPPED_LIST[@]}" -gt 0 ]; then
    echo " SKIPPED:"
    for s in "${SKIPPED_LIST[@]}"; do echo "   ↷ $s"; done
fi
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
