#!/bin/bash
# Isolate ClawEnv state to a test-scoped $HOME directory.
#
# Why $HOME override: v2/core/src/* uses `dirs::home_dir()` which reads
# $HOME on macOS/Linux. Setting HOME=/tmp/clawenv-e2e-<ts> reroutes
# every clawenv path — config.toml, instances.toml, lima VMs,
# workspaces, cache — into the isolated tree. Zero code change needed.
# Restores after teardown.
#
# Sourced by run.sh before any clawcli invocation.
#
# Lifted verbatim from v1 tests/e2e/lib/isolate.sh — no v2 changes
# needed because the path-isolation mechanism is identical.

# HARD SAFETY INVARIANT: teardown only ever deletes paths matching this glob.
# A prior version had an unguarded `rm -rf "$test_home"` that, when $HOME was
# clobbered during a kill-handler trap, walked up and destroyed the entire
# workspace and unrelated user files on a TRIM-enabled APFS SSD. The
# whitelist below is the load-bearing fix. Do not remove.
: "${E2E_TEARDOWN_ALLOWED_PREFIX:=/tmp/clawenv-e2e-}"
if [ -z "${_E2E_TEARDOWN_PREFIX_LOCKED:-}" ]; then
    readonly E2E_TEARDOWN_ALLOWED_PREFIX
    _E2E_TEARDOWN_PREFIX_LOCKED=1
fi

e2e_isolate_setup() {
    local ext="${E2E_TEST_HOME:-}"
    if [ -n "$ext" ] && [ "${ext#$E2E_TEARDOWN_ALLOWED_PREFIX}" = "$ext" ]; then
        echo "[isolate] refusing E2E_TEST_HOME='$ext' — must begin with $E2E_TEARDOWN_ALLOWED_PREFIX" >&2
        exit 2
    fi
    local default_suffix
    if [ -n "${E2E_HOME_SUFFIX:-}" ]; then
        default_suffix="${E2E_HOME_SUFFIX}"
    else
        default_suffix="$(date +%s)-$$"
    fi
    export E2E_TEST_HOME="${ext:-${E2E_TEARDOWN_ALLOWED_PREFIX}${default_suffix}}"
    mkdir -p "$E2E_TEST_HOME"
    export E2E_REAL_HOME="${HOME}"
    export HOME="$E2E_TEST_HOME"
    echo "[isolate] HOME=$HOME (was $E2E_REAL_HOME)" >&2
}

# Validate that a path is safe for `rm -rf`. Returns 0 if safe.
_e2e_rm_safe() {
    local p="$1"
    if [ -z "$p" ]; then
        echo "[isolate:safety] empty path — refuse rm" >&2; return 1
    fi
    case "$p" in
        /) echo "[isolate:safety] root '/' — refuse rm" >&2; return 1 ;;
        *..* ) echo "[isolate:safety] '..' anywhere in path '$p' — refuse rm" >&2; return 1 ;;
    esac
    if [ "${p#$E2E_TEARDOWN_ALLOWED_PREFIX}" = "$p" ]; then
        echo "[isolate:safety] path '$p' outside whitelist '$E2E_TEARDOWN_ALLOWED_PREFIX*' — refuse rm" >&2
        return 1
    fi
    if [ -L "$p" ]; then
        echo "[isolate:safety] '$p' is a symlink — refuse rm" >&2
        return 1
    fi
    return 0
}

e2e_isolate_teardown() {
    local keep="${1:-0}"
    local test_home="${E2E_TEST_HOME:-}"
    if [ -n "${E2E_REAL_HOME:-}" ]; then
        export HOME="$E2E_REAL_HOME"
    fi

    if ! _e2e_rm_safe "$test_home"; then
        echo "[isolate] ABORT teardown — unsafe path, leaving it for manual cleanup" >&2
        return 1
    fi

    if [ "$keep" = "1" ]; then
        echo "[isolate] keeping $test_home for post-mortem" >&2
        return 0
    fi

    # Kill any Lima VMs that might still reference this test home, then rm.
    if [ -d "$test_home/.clawenv/lima" ]; then
        LIMA_HOME="$test_home/.clawenv/lima" \
            "$E2E_REAL_HOME/.clawenv/bin/limactl" list 2>/dev/null | \
            awk 'NR>1 {print $1}' | \
            while read -r vm; do
                LIMA_HOME="$test_home/.clawenv/lima" \
                    "$E2E_REAL_HOME/.clawenv/bin/limactl" delete --force "$vm" 2>/dev/null || true
            done
    fi
    if _e2e_rm_safe "$test_home"; then
        rm -rf -- "$test_home"
        echo "[isolate] removed $test_home" >&2
    else
        echo "[isolate] ABORT final rm — path became unsafe mid-teardown" >&2
        return 1
    fi
}

# Dump test log to ~/Desktop/ClawEnv-v2/logs/ so it survives teardown.
e2e_archive_log() {
    local scenario="$1"
    local status="$2"  # pass|fail|skip
    local logs_dir="$E2E_REAL_HOME/Desktop/ClawEnv-v2/logs"
    mkdir -p "$logs_dir"
    local ts=$(date +%Y%m%d-%H%M%S)
    local dest="$logs_dir/${scenario}-${status}-${ts}.log"
    if [ -f "$E2E_TEST_HOME/clawenv-e2e.log" ]; then
        cp "$E2E_TEST_HOME/clawenv-e2e.log" "$dest"
        echo "[isolate] log archived → $dest" >&2
    fi
}
