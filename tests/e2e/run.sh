#!/bin/bash
# ClawEnv E2E test runner.
# Usage:
#   ./tests/e2e/run.sh all
#   ./tests/e2e/run.sh smoke-mac-sandbox-noproxy
#   ./tests/e2e/run.sh --skip-proxy all
#   ./tests/e2e/run.sh --home-suffix 01 <scenario>    # parallel-safe
#   ./tests/e2e/run.sh --parallel --with-windows all  # fork per-scenario
# See docs/24-e2e-testing.md for the smoke-test matrix.

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
# --parallel mode: spawn one child run.sh per scenario with a distinct
# --home-suffix so their isolated /tmp/clawenv-e2e-<suffix> dirs don't
# collide. Default serial mode stays unchanged. Prints a status line
# every 60s while children are in flight.
E2E_PARALLEL="0"
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
        --parallel)
            E2E_PARALLEL="1"
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
source "$E2E_REPO_ROOT/tests/e2e/lib/preflight.sh"

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

# In --parallel mode the PARENT doesn't isolate its own HOME — it
# would leak E2E_TEST_HOME + HOME into the forked children, and every
# child would then reuse the parent's dir, teardown-racing each other
# at exit. Each child runs its own isolate_setup → teardown pair with
# the --home-suffix we pass below. Serial mode below keeps the
# parent's setup+teardown as before.
if [ "$E2E_PARALLEL" != "1" ]; then
    # ──────────────────────────────────────────────────────────────
    # Setup: isolate home (serial mode only)
    # ──────────────────────────────────────────────────────────────
    e2e_isolate_setup
    # Seed isolated HOME with cached lima/git/node from real ~/.clawenv.
    # Each concurrent run gets its own HOME (E2E_HOME_SUFFIX) so the
    # copies don't collide. See lib/prewarm.sh for what gets copied.
    e2e_prewarm_seed_home

    cleanup() {
        local rc=$?
        e2e_isolate_teardown "$E2E_KEEP_HOME"
        exit $rc
    }
    trap cleanup EXIT INT TERM
fi

# ──────────────────────────────────────────────────────────────
# Parallel mode: fork one child run.sh per scenario with a distinct
# --home-suffix, then poll every 60s until all children exit. Each
# child runs its scenarios serially (so with one scenario per child
# you get full parallelism across scenarios).
#
# Reasons this lives at the parent-run.sh level rather than as a
# separate `orchestrator.sh`: (a) a user running `run.sh --parallel
# all` gets parallel execution without hunting for another script;
# (b) the Windows `sync + remote-build` step needs to run ONCE for
# all Windows scenarios, not once per scenario, and doing it here
# keeps that invariant explicit.
# ──────────────────────────────────────────────────────────────
if [ "$E2E_PARALLEL" = "1" ] && [ "${#SELECTION[@]}" -gt 1 ]; then
    echo ""
    echo "════════════════════════════════════════════════════════════"
    echo "▶ Parallel mode: ${#SELECTION[@]} scenarios, polling every 60s"
    echo "════════════════════════════════════════════════════════════"

    # Once-per-run Windows prep: if any scenario needs Windows, kill
    # lingering clawcli.exe on the VM (holds target/release/clawcli.exe
    # open), rsync source, then remote-build. Skip entirely if no
    # Windows scenario is selected — keeps mac-only runs fast.
    WANTS_WIN="0"
    for n in "${SELECTION[@]}"; do
        if [[ "$n" == *-win-* ]]; then WANTS_WIN="1"; break; fi
    done
    if [ "$WANTS_WIN" = "1" ]; then
        echo "[parallel] Windows scenarios present — sync + remote-build"
        ssh -o ConnectTimeout=10 "${WIN_USER:-clawenv}@${WIN_HOST:-192.168.64.7}" \
            "taskkill /F /IM clawcli.exe /T" >/dev/null 2>&1 || true
        if ! bash "$E2E_REPO_ROOT/scripts/win-remote.sh" sync > "/tmp/clawenv-e2e-win-sync.log" 2>&1; then
            echo "[parallel] win sync failed — see /tmp/clawenv-e2e-win-sync.log" >&2
            exit 3
        fi
        if ! bash "$E2E_REPO_ROOT/scripts/win-remote.sh" run \
            'C:\Users\clawenv\.cargo\bin\cargo.exe build -p clawcli --release' \
            > "/tmp/clawenv-e2e-win-build.log" 2>&1
        then
            echo "[parallel] win build failed — see /tmp/clawenv-e2e-win-build.log" >&2
            exit 3
        fi
        # Clear every ClawEnv-e2e* dir on the Windows side, not just
        # the bare name — parallel children suffix their CLAWENV_HOME
        # with the scenario suffix (e.g. ClawEnv-e2e-wn0 / ClawEnv-e2e-wn1).
        # Keep glob-compatible; cmd.exe's `rmdir /s /q *pattern*` doesn't
        # accept wildcards, so enumerate via dir.
        bash "$E2E_REPO_ROOT/scripts/win-remote.sh" run \
            'for /d %i in ("%USERPROFILE%\Desktop\ClawEnv-e2e*") do @rmdir /s /q "%i"' \
            > /dev/null 2>&1 || true
    fi

    # Short suffix per scenario — Lima's UNIX socket path is capped at
    # 104 chars and the full HOME path feeds into it, so we use 3-char
    # codes rather than the descriptive scenario name.
    # Case statement (not associative array) because macOS ships
    # bash 3.2 which lacks `declare -A`.
    scenario_suffix() {
        case "$1" in
            smoke-mac-native-noproxy)       echo "mn0" ;;
            smoke-mac-native-http-proxy)    echo "mn1" ;;
            smoke-mac-sandbox-noproxy)      echo "ms0" ;;
            smoke-mac-sandbox-http-proxy)   echo "ms1" ;;
            smoke-win-native-noproxy)       echo "wn0" ;;
            smoke-win-native-http-proxy)    echo "wn1" ;;
            smoke-linux-podman-noproxy)     echo "lp0" ;;
            smoke-linux-podman-http-proxy)  echo "lp1" ;;
            smoke-mac-import-export)        echo "mx0" ;;
            smoke-mac-upgrade)              echo "mu0" ;;
            # Defensive fallback: take first 6 chars of the stripped name.
            *) local s="${1#smoke-}"; echo "${s:0:6}" ;;
        esac
    }

    LOGDIR="/tmp/clawenv-e2e-parallel-$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$LOGDIR"
    echo "[parallel] per-scenario logs -> $LOGDIR"

    # Pre-clean any orphan isolated HOMEs from prior runs, but keep
    # -prewarm (expensive to rebuild) and the socks-pass log.
    for d in /tmp/clawenv-e2e-*; do
        [ -e "$d" ] || continue
        case "$(basename "$d")" in
            clawenv-e2e-prewarm|clawenv-e2e-socks-pass.log|clawenv-e2e-parallel-*|clawenv-e2e-win-*) continue ;;
        esac
        chmod -R u+w "$d" 2>/dev/null || true
        rm -rf "$d"
    done

    # `declare -a` is also bash 3.2-safe; keep it explicit for clarity.
    PIDS=()
    NAMES=()
    PAR_STARTS=()
    PARALLEL_STARTED=$(date +%s)
    for name in "${SELECTION[@]}"; do
        suffix="$(scenario_suffix "$name")"
        extra_flags=""
        [[ "$name" == *-win-* ]] && extra_flags="--with-windows"
        log="$LOGDIR/${name}.log"
        echo "[parallel] launch: $name (suffix=$suffix)"
        ( "$0" --home-suffix "$suffix" $extra_flags "$name" ) > "$log" 2>&1 &
        PIDS+=("$!")
        NAMES+=("$name")
        PAR_STARTS+=("$(date +%s)")
    done

    tick=0
    while :; do
        sleep 60
        tick=$((tick+1))
        now=$(date +%s)
        wall=$((now - PARALLEL_STARTED))
        running=0
        status_line=""
        for i in "${!PIDS[@]}"; do
            if kill -0 "${PIDS[$i]}" 2>/dev/null; then
                running=$((running+1))
                elap=$((now - PAR_STARTS[$i]))
                status_line+="  RUN ${NAMES[$i]} (${elap}s)"$'\n'
            fi
        done
        echo ""
        echo "━━ tick $tick  wall=${wall}s  running=${running}/${#PIDS[@]} ━━"
        [ "$running" -gt 0 ] && printf '%s' "$status_line"
        [ "$running" -eq 0 ] && break
    done

    TOTAL_ELAPSED=$(( $(date +%s) - PARALLEL_STARTED ))
    PASS=0; FAIL=0; SKIP=0
    FAILED_NAMES=()
    SKIPPED_NAMES=()
    echo ""
    echo "════════════════════════════════════════════════════════════"
    echo " Parallel Summary — wall=${TOTAL_ELAPSED}s"
    echo "════════════════════════════════════════════════════════════"
    for i in "${!PIDS[@]}"; do
        wait "${PIDS[$i]}"; rc=$?
        case $rc in
            0)  echo "  ✓ PASS    ${NAMES[$i]}"; PASS=$((PASS+1)) ;;
            77) echo "  ↷ SKIP    ${NAMES[$i]}"; SKIP=$((SKIP+1)); SKIPPED_NAMES+=("${NAMES[$i]}") ;;
            *)  echo "  ✗ FAIL    ${NAMES[$i]} (rc=$rc)"; FAIL=$((FAIL+1)); FAILED_NAMES+=("${NAMES[$i]}") ;;
        esac
    done
    echo ""
    echo "  totals: pass=$PASS fail=$FAIL skip=$SKIP"
    echo "  logs:   $LOGDIR"
    echo "════════════════════════════════════════════════════════════"
    # We do NOT run the serial trap's isolate_teardown here because each
    # child ran its own isolate setup/teardown pair.
    [ "$FAIL" -eq 0 ] && exit 0 || exit 1
fi

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
        source "$E2E_REPO_ROOT/tests/e2e/lib/preflight.sh"
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
