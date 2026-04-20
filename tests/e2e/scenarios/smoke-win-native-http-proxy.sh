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

echo ">> probe net-check on Windows via $E2E_WIN_HTTP_PROXY" >&2
cli_win_with_proxy() {
    local old_prefix="$WIN_ENV_PREFIX"
    export WIN_ENV_PREFIX="set HTTP_PROXY=${E2E_WIN_HTTP_PROXY}&&set HTTPS_PROXY=${E2E_WIN_HTTP_PROXY}&&${old_prefix}"
    cli_win "$@"
    local rc=$?
    export WIN_ENV_PREFIX="$old_prefix"
    return $rc
}
cli_win_with_proxy "net-check --mode native --probe host,npm,git --proxy-url \"$E2E_WIN_HTTP_PROXY\""
_ok "win-native net probes pass via HTTP proxy"
