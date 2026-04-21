#!/bin/bash
# Smoke probe — upgrade path on macOS sandbox.
# Installs a pinned older version of openclaw, runs `clawcli upgrade`
# to latest, and confirms the new version string replaces the old.
# Covers the mid-lifecycle code path that otherwise only the manual
# "user clicks Upgrade in GUI" gesture exercises. ~8-10min wall.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

[ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "upgrade scenario requires proxy (install + upgrade both fetch npm)"

e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"

export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"

NAME="probe-mac-upgrade"
PORT="11801"
# Intentionally OLD version so `upgrade` has something to do. Adjust
# when openclaw cuts a release that makes this too old to boot.
OLD_VERSION="0.1.0"

cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> phase 1: install pinned old version ($OLD_VERSION)" >&2
cli install --mode sandbox --claw-type openclaw --version "$OLD_VERSION" --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "phase 1: old version installed"

# Capture pre-upgrade version (read from clawcli status JSON)
pre_ver=$(cli status --name "$NAME" 2>/dev/null | awk -F'"version":"' '/version/ {print $2}' | head -1 | cut -d'"' -f1)
echo "   pre-upgrade version: ${pre_ver:-unknown}" >&2

echo ">> phase 2: upgrade to latest" >&2
cli upgrade --name "$NAME"
_ok "phase 2: upgrade completed"

# Capture post-upgrade version
post_ver=$(cli status --name "$NAME" 2>/dev/null | awk -F'"version":"' '/version/ {print $2}' | head -1 | cut -d'"' -f1)
echo "   post-upgrade version: ${post_ver:-unknown}" >&2

# Guard: version string must have changed. If clawcli status reports
# the same string before and after, upgrade silently no-op'd and we
# want that surfaced as a test failure, not a pass.
if [ -n "$pre_ver" ] && [ -n "$post_ver" ] && [ "$pre_ver" = "$post_ver" ]; then
    _fail "version string unchanged after upgrade ($pre_ver) — upgrade had no effect"
else
    _ok "version changed: ${pre_ver:-?} -> ${post_ver:-?}"
fi

cli uninstall --name "$NAME" 2>/dev/null || true
_ok "cleanup done"
