#!/bin/bash
# Smoke probe — macOS Native, HTTP proxy (host OS).
# Validates reqwest + npm + git all honour HTTP_PROXY when set.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

[ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "macOS HTTPEnable=0 — no HTTP proxy configured"

# Verify the proxy actually works before committing to the full probe.
# Fails fast (exit 2) if the configured proxy can't reach registry.
e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"

export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"

NAME="probe-mn1"

cli uninstall --name "$NAME" 2>/dev/null || true

# Install ClawEnv-native toolchain under the HTTP proxy — node + git
# download goes via the same proxy that's being validated. Required so
# the subsequent net-check has an honest clawenv-native node/git to
# probe against (see has_node/has_git gate in run_native_probe).
echo ">> step prereq (install clawenv-native node + git via HTTP proxy)" >&2
cli install --mode native --claw-type openclaw --version latest \
    --name "$NAME" --port 11404 --step prereq
_ok "clawenv-native prereq ready"

echo ">> probe net-check (host+npm+git via HTTP $E2E_MAC_HTTP_PROXY)" >&2
cli net-check --mode native --probe host,npm,git --proxy-url "$E2E_MAC_HTTP_PROXY"
_ok "native net probes pass via HTTP proxy"

cli uninstall --name "$NAME" 2>/dev/null || true
