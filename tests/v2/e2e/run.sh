#!/bin/bash
# ClawEnv v2 E2E test runner.
#
# Usage:
#   ./tests/v2/e2e/run.sh all
#   ./tests/v2/e2e/run.sh smoke-mac-sandbox-noproxy
#   ./tests/v2/e2e/run.sh --skip-proxy all
#   ./tests/v2/e2e/run.sh --keep-home <scenario>     # leave HOME on disk
#
# Adapted from v1 tests/e2e/run.sh. v2 differences:
# - Repo root resolves to .../ClawEnv (the parent of v2/), so the lib
#   helpers can find target/release/clawcli under target/.
# - No Windows fork (--with-windows / --windows-only): the v2 win-rsync
#   harness lands in a follow-up.
# - No --parallel mode yet: simpler serial loop until we need it.

set -eu

# Repo root = three parents up from this script (ClawEnv/tests/v2/e2e/).
export E2E_REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"

export E2E_BUNDLE_DIR="${E2E_BUNDLE_DIR:-$HOME/Desktop/clawenv-v2/exports}"
E2E_KEEP_HOME="0"
E2E_SKIP_PROXY="0"
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
        --verbose)
            export E2E_VERBOSE="1"
            shift ;;
        -h|--help)
            head -10 "$0" | tail -8
            exit 0 ;;
        *)
            SELECTION+=("$1")
            shift ;;
    esac
done

if [ "${#SELECTION[@]}" -eq 0 ]; then
    echo "Usage: $0 [options] all|<scenario-name>" >&2
    echo "Scenarios:" >&2
    ls "$E2E_REPO_ROOT/tests/v2/e2e/scenarios/" | sed 's/\.sh$//' | sed 's/^/  /' >&2
    exit 2
fi

# Expand `all`. Skips Windows scenarios entirely (deferred).
if [ "${SELECTION[0]}" = "all" ]; then
    SELECTION=()
    for f in "$E2E_REPO_ROOT/tests/v2/e2e/scenarios/"*.sh; do
        [ -f "$f" ] || continue
        name=$(basename "$f" .sh)
        [[ "$name" == *windows* || "$name" == *-win-* ]] && continue
        if [ "$E2E_SKIP_PROXY" = "1" ] && [[ "$name" == *-proxy ]]; then continue; fi
        SELECTION+=("$name")
    done
fi

# ──────────────────────────────────────────────────────────────
# Source libraries
# ──────────────────────────────────────────────────────────────
source "$E2E_REPO_ROOT/tests/v2/e2e/lib/isolate.sh"
source "$E2E_REPO_ROOT/tests/v2/e2e/lib/cli.sh"
source "$E2E_REPO_ROOT/tests/v2/e2e/lib/assert.sh"
source "$E2E_REPO_ROOT/tests/v2/e2e/lib/detect-proxy.sh"
source "$E2E_REPO_ROOT/tests/v2/e2e/lib/prewarm.sh"
source "$E2E_REPO_ROOT/tests/v2/e2e/lib/preflight.sh"

# ──────────────────────────────────────────────────────────────
# Preflight
# ──────────────────────────────────────────────────────────────
if ! command -v jq >/dev/null; then
    echo "✗ jq not installed. \`brew install jq\`" >&2
    exit 3
fi

detect_mac_http_proxy
detect_mac_socks_proxy
detect_proxy_summary

CLI_BIN="$(e2e_cli_bin)"
if [ ! -x "$CLI_BIN" ] && ! command -v "$CLI_BIN" >/dev/null; then
    echo "✗ clawcli not built. Run \`cargo build -p clawops-cli --release\` from v2/." >&2
    echo "   (or set CLAWCLI_BIN to override.)" >&2
    exit 3
fi
echo "[runner] clawcli = $CLI_BIN" >&2
echo "[runner] exports = $E2E_BUNDLE_DIR" >&2
mkdir -p "$E2E_BUNDLE_DIR" 2>/dev/null || true

e2e_isolate_setup
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
    script="$E2E_REPO_ROOT/tests/v2/e2e/scenarios/${name}.sh"

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

    rc=0
    (
        set -eu
        source "$E2E_REPO_ROOT/tests/v2/e2e/lib/isolate.sh"
        source "$E2E_REPO_ROOT/tests/v2/e2e/lib/cli.sh"
        source "$E2E_REPO_ROOT/tests/v2/e2e/lib/assert.sh"
        source "$E2E_REPO_ROOT/tests/v2/e2e/lib/detect-proxy.sh"
        source "$E2E_REPO_ROOT/tests/v2/e2e/lib/prewarm.sh"
        source "$E2E_REPO_ROOT/tests/v2/e2e/lib/preflight.sh"
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
