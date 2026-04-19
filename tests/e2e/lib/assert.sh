#!/bin/bash
# Assertion helpers. Each prints a green ✓ on success, red ✗ + context
# on failure, and returns 0/1 accordingly. Scenarios should `set -e` so
# any failed assertion aborts.

e2e_assert_count=0
e2e_assert_passed=0

e2e_assert_init() { e2e_assert_count=0; e2e_assert_passed=0; }

_ok()   { e2e_assert_count=$((e2e_assert_count+1)); e2e_assert_passed=$((e2e_assert_passed+1)); echo "  ✓ $1" >&2; return 0; }
_fail() { e2e_assert_count=$((e2e_assert_count+1)); echo "  ✗ $1" >&2; return 1; }

# Wait for URL to return HTTP 200. Used for gateway health checks.
expect_http_200() {
    local url="$1"
    local timeout="${2:-60}"
    local deadline=$(($(date +%s) + timeout))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        local code
        code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "$url" 2>/dev/null || echo 000)
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
    # Glob support: expand and check first match.
    set -- $path
    if [ -f "$1" ] && [ -s "$1" ]; then
        _ok "file exists: $1 ($(stat -f %z "$1") bytes)"
        return 0
    fi
    _fail "file missing or empty: $path"
    return 1
}

# Lima VM is running. Needs LIMA_HOME env set properly.
expect_limactl_running() {
    local name_pattern="${1:-clawenv-}"
    local lima
    lima="$E2E_REAL_HOME/.clawenv/bin/limactl"
    if [ ! -x "$lima" ]; then
        _fail "limactl not found at $lima"
        return 1
    fi
    if LIMA_HOME="$E2E_TEST_HOME/.clawenv/lima" "$lima" list 2>/dev/null | \
        awk 'NR>1 {print $1, $2}' | \
        grep -q "^${name_pattern}.* Running$"; then
        _ok "limactl running: $name_pattern"
        return 0
    fi
    _fail "no Running Lima VM matching '$name_pattern'"
    LIMA_HOME="$E2E_TEST_HOME/.clawenv/lima" "$lima" list 2>&1 | head -5 >&2
    return 1
}

# No Lima VM matching pattern (used post-uninstall).
expect_no_limactl() {
    local name_pattern="${1:-clawenv-}"
    local lima
    lima="$E2E_REAL_HOME/.clawenv/bin/limactl"
    if LIMA_HOME="$E2E_TEST_HOME/.clawenv/lima" "$lima" list 2>/dev/null | \
        awk 'NR>1 {print $1}' | \
        grep -q "^${name_pattern}"; then
        _fail "Lima VM still present matching '$name_pattern'"
        return 1
    fi
    _ok "no Lima VM matches '$name_pattern'"
    return 0
}

# config.toml has an [[instances]] entry with given name.
expect_config_entry() {
    local name="$1"
    local cfg="$E2E_TEST_HOME/.clawenv/config.toml"
    if [ ! -f "$cfg" ]; then
        _fail "config.toml missing: $cfg"
        return 1
    fi
    if grep -q "^name = \"$name\"$" "$cfg"; then
        _ok "config has instance '$name'"
        return 0
    fi
    _fail "config.toml has no instance '$name'"
    return 1
}

expect_no_config_entry() {
    local name="$1"
    local cfg="$E2E_TEST_HOME/.clawenv/config.toml"
    if [ ! -f "$cfg" ]; then
        _ok "config.toml missing (expected — no instances)"
        return 0
    fi
    if grep -q "^name = \"$name\"$" "$cfg"; then
        _fail "config.toml still has instance '$name'"
        return 1
    fi
    _ok "config has no instance '$name'"
    return 0
}

# Bundle tarball has a valid clawenv-bundle.toml manifest at root.
expect_bundle_manifest() {
    local path="$1"
    if [ ! -f "$path" ]; then
        _fail "bundle missing: $path"
        return 1
    fi
    # Peek manifest via `tar -xzf -O`.
    local manifest
    manifest=$(tar -xzf "$path" -O clawenv-bundle.toml 2>/dev/null)
    if [ -z "$manifest" ]; then
        _fail "bundle $path has no clawenv-bundle.toml"
        return 1
    fi
    if echo "$manifest" | grep -q '^schema_version = 1$'; then
        _ok "bundle manifest valid (schema v1)"
        return 0
    fi
    _fail "bundle manifest malformed:"
    echo "$manifest" | head -10 >&2
    return 1
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
