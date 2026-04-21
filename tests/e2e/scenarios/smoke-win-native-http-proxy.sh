#!/bin/bash
# Smoke probe — Windows Native, HTTP proxy (Windows's own).

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

source "$E2E_REPO_ROOT/tests/e2e/lib/win-remote.sh"
e2e_win_load_env || exit 3

e2e_assert_init

[ -n "${E2E_WIN_HTTP_PROXY:-}" ] || _skip "Windows ProxyEnable=0 in registry — no HTTP proxy"

# Verify the Windows HTTP proxy works FROM WINDOWS. The proxy URL we
# get from Windows' registry (e.g. http://127.0.0.1:10808) refers to a
# loopback daemon ON THE WINDOWS VM — dialing it from the driving Mac
# would hit Mac's own loopback (wrong host). SSH over and curl there.
e2e_preflight_proxy_on_win "$E2E_WIN_HTTP_PROXY"

NAME="probe-wn1"

cli_win_with_proxy() {
    local old_prefix="$WIN_ENV_PREFIX"
    export WIN_ENV_PREFIX="set HTTP_PROXY=${E2E_WIN_HTTP_PROXY}&&set HTTPS_PROXY=${E2E_WIN_HTTP_PROXY}&&${old_prefix}"
    cli_win "$@"
    local rc=$?
    export WIN_ENV_PREFIX="$old_prefix"
    return $rc
}

cli_win "uninstall --name \"$NAME\"" 2>/dev/null || true

echo ">> step prereq on Windows (install clawenv-native node + git via HTTP proxy)" >&2
cli_win_with_proxy "install --mode native --claw-type openclaw --version latest --name \"$NAME\" --port 11504 --step prereq"
_ok "clawenv-native prereq ready (Windows, via proxy)"

echo ">> probe net-check on Windows via $E2E_WIN_HTTP_PROXY" >&2
cli_win_with_proxy "net-check --mode native --probe host,npm,git --proxy-url \"$E2E_WIN_HTTP_PROXY\""
_ok "win-native net probes pass via HTTP proxy"

cli_win "uninstall --name \"$NAME\"" 2>/dev/null || true
