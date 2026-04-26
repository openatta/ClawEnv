#!/bin/bash
# Assertion helpers. Each prints a green ✓ on success, red ✗ + context
# on failure, and returns 0/1 accordingly. Scenarios should `set -e` so
# any failed assertion aborts.
#
# Adapted from v1 tests/e2e/lib/assert.sh. v2-specific changes:
# - expect_config_entry / expect_no_config_entry now check
#   `instances.toml` (v2's source of truth), not `config.toml`.

e2e_assert_count=0
e2e_assert_passed=0

e2e_assert_init() { e2e_assert_count=0; e2e_assert_passed=0; }

_ok()   { e2e_assert_count=$((e2e_assert_count+1)); e2e_assert_passed=$((e2e_assert_passed+1)); echo "  ✓ $1" >&2; return 0; }
_fail() { e2e_assert_count=$((e2e_assert_count+1)); echo "  ✗ $1" >&2; return 1; }
# `_skip <reason>` — exit with GNU SKIP code (77).
_skip() { echo "  ↷ skipped: $1" >&2; exit 77; }

# Wait for URL to return HTTP 200. Used for gateway health checks.
# `--noproxy '*'` is critical: a system HTTPS_PROXY (commonly used for
# install-time fetches) MUST NOT be applied when probing 127.0.0.1
# loopback. Without --noproxy, the local proxy daemon will tunnel back
# to its own 127.0.0.1:<port> on the same machine, hiding cases where
# the daemon has actually died (proxy-side keep-alive masks it).
expect_http_200() {
    local url="$1"
    local timeout="${2:-60}"
    local deadline=$(($(date +%s) + timeout))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        local code
        code=$(curl -s --noproxy '*' -o /dev/null -w '%{http_code}' --max-time 3 "$url" 2>/dev/null || echo 000)
        if [ "$code" = "200" ] || [ "$code" = "301" ] || [ "$code" = "302" ]; then
            _ok "HTTP $code from $url"
            return 0
        fi
        sleep 2
    done
    _fail "no HTTP 200 from $url within ${timeout}s"
    return 1
}

# File exists + size > 0.
expect_file() {
    local path="$1"
    set -- $path
    if [ -f "$1" ] && [ -s "$1" ]; then
        local sz
        if [ "$(uname -s)" = "Darwin" ]; then
            sz=$(stat -f %z "$1")
        else
            sz=$(stat -c %s "$1")
        fi
        _ok "file exists: $1 (${sz} bytes)"
        return 0
    fi
    _fail "file missing or empty: $path"
    return 1
}

# Instance's sandbox VM is running. Delegates to `clawcli status`
# (which uses the right path resolution under the isolated $HOME)
# rather than shelling out to limactl directly with hardcoded paths.
# v2 stores limactl at `$HOME/.clawenv/bin/limactl` (test home, not
# real home) and Lima itself uses `$HOME/.lima/` — both float with
# `$HOME`, so going through clawcli is the only correct path.
expect_limactl_running() {
    local name="$1"
    [ -n "$name" ] || { _fail "expect_limactl_running: instance name required"; return 1; }
    local state
    state=$("$(e2e_cli_bin)" --json status "$name" 2>/dev/null \
        | jq -r 'select(.type=="data") | .data.vm.state // empty' | tail -1)
    if [ "$state" = "running" ]; then
        _ok "instance '$name' VM running"
        return 0
    fi
    _fail "instance '$name' VM not running (state=${state:-?})"
    return 1
}

# No registered VM for instance (used post-destroy).
expect_no_limactl() {
    local name="$1"
    [ -n "$name" ] || { _fail "expect_no_limactl: instance name required"; return 1; }
    # If the instance is unregistered, status synthesises a view with
    # registered=false. If the registry still has it, registered=true.
    local registered
    registered=$("$(e2e_cli_bin)" --json status "$name" 2>/dev/null \
        | jq -r 'select(.type=="data") | .data.registered // empty' | tail -1)
    if [ "$registered" = "false" ] || [ -z "$registered" ]; then
        _ok "no registered VM for '$name'"
        return 0
    fi
    _fail "instance '$name' still registered"
    return 1
}

# v2 instance registry has an entry with given name.
# v2 stores instances in $HOME/.clawenv/v2/instances.toml as
#   [[instance]] name = "..." claw = "..." backend = "..."
# (the v2/ subdir is per `paths::v2_config_dir()`)
expect_config_entry() {
    local name="$1"
    local cfg="$E2E_TEST_HOME/.clawenv/v2/instances.toml"
    if [ ! -f "$cfg" ]; then
        _fail "instances.toml missing: $cfg"
        return 1
    fi
    if grep -q "^name = \"$name\"$" "$cfg"; then
        _ok "registry has instance '$name'"
        return 0
    fi
    _fail "instances.toml has no instance '$name'"
    return 1
}

expect_no_config_entry() {
    local name="$1"
    local cfg="$E2E_TEST_HOME/.clawenv/v2/instances.toml"
    if [ ! -f "$cfg" ]; then
        _ok "instances.toml missing (expected — no instances)"
        return 0
    fi
    if grep -q "^name = \"$name\"$" "$cfg"; then
        _fail "instances.toml still has instance '$name'"
        return 1
    fi
    _ok "registry has no instance '$name'"
    return 0
}

# Print assertion summary. Returns 0 if all passed.
e2e_assert_summary() {
    echo "" >&2
    if [ "$e2e_assert_count" -eq "$e2e_assert_passed" ]; then
        echo "=== ${e2e_assert_passed}/${e2e_assert_count} assertions passed ===" >&2
        return 0
    fi
    echo "=== ${e2e_assert_passed}/${e2e_assert_count} assertions passed (FAIL) ===" >&2
    return 1
}
