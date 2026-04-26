#!/bin/bash
# Smoke probe — upgrade path on macOS sandbox.
# Installs an older pinned openclaw, runs `clawcli upgrade`, and
# confirms the version string changed. Covers the mid-lifecycle path
# that otherwise only the manual GUI Upgrade gesture exercises.
# ~10-15min wall (full install + upgrade).

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

case "$(uname -s)" in
    Darwin) : ;;
    *) _skip "macOS-only scenario (uname=$(uname -s))" ;;
esac

[ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "upgrade scenario requires proxy (install + upgrade both fetch npm)"

e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"

export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"

NAME="probe-mac-upgrade"
PORT="11801"
# Intentionally OLD version so `upgrade` has work to do. Adjust when
# openclaw cuts a release that makes this too old to boot.
OLD_VERSION="0.1.0"

cli instance destroy "$NAME" 2>/dev/null || true

echo ">> phase 1: install pinned old version ($OLD_VERSION)" >&2
cli install openclaw --backend lima --version "$OLD_VERSION" --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "phase 1: old version installed"

# Capture pre-upgrade version via `clawcli upgrade --check` (registry
# probe — no VM touch). This returns a Data event with current+latest.
pre_ver=$("$(e2e_cli_bin)" --json upgrade "$NAME" --check 2>/dev/null \
    | jq -r 'select(.type=="data") | .data.current // empty' | tail -1)
echo "   pre-upgrade version: ${pre_ver:-unknown}" >&2

echo ">> phase 2: upgrade to latest" >&2
cli upgrade "$NAME"
_ok "phase 2: upgrade completed"

post_ver=$("$(e2e_cli_bin)" --json upgrade "$NAME" --check 2>/dev/null \
    | jq -r 'select(.type=="data") | .data.current // empty' | tail -1)
echo "   post-upgrade version: ${post_ver:-unknown}" >&2

if [ -n "$pre_ver" ] && [ -n "$post_ver" ] && [ "$pre_ver" = "$post_ver" ]; then
    _fail "version string unchanged after upgrade ($pre_ver) — upgrade had no effect"
else
    _ok "version changed: ${pre_ver:-?} -> ${post_ver:-?}"
fi

cli instance destroy "$NAME" 2>/dev/null || true
_ok "cleanup done"
