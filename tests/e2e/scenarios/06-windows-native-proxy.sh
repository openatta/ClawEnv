#!/bin/bash
# Scenario 06: native install on Windows ARM64 with proxy.
#
# Design note: the Mac-side mini_proxy listens on 127.0.0.1:$E2E_PROXY_LISTEN,
# but Windows VM's 127.0.0.1 is its own loopback — can't reach the Mac.
# We pass the Mac's UTM-visible IP (resolvable from Windows guest) as the
# proxy host. Discovery order:
#   1. E2E_WIN_HOST_PROXY env (user-supplied override)
#   2. Read the default gateway of the Windows box (UTM provides this
#      as the Mac's virtual NIC IP — same address Windows uses for
#      outbound)
#
# If mini_proxy isn't reachable from Windows (e.g. firewall), the test
# fails on the first install call with a clear error.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

source "$E2E_REPO_ROOT/tests/e2e/lib/win-remote.sh"
e2e_win_load_env || exit 3

NAME="e2e-win-nat-proxy"
PORT="10300"
WIN_BUNDLE="%USERPROFILE%\\Desktop\\ClawEnv\\${NAME}-$(date +%Y%m%d-%H%M%S).tar.gz"

e2e_assert_init

# Resolve Mac's IP as seen from Windows. UTM default network puts the
# Mac at the gateway address of the Windows NIC.
MAC_IP="${E2E_WIN_HOST_PROXY:-}"
if [ -z "$MAC_IP" ]; then
    MAC_IP=$(win_exec "powershell -NoProfile -Command \"(Get-NetRoute -DestinationPrefix 0.0.0.0/0 | Select-Object -First 1).NextHop\"" 2>/dev/null | tail -1 | tr -d '\r\n')
    if [ -z "$MAC_IP" ]; then
        echo "✗ Could not resolve Mac IP from Windows. Set E2E_WIN_HOST_PROXY to Mac's UTM IP." >&2
        exit 4
    fi
fi
WIN_PROXY_URL="http://${MAC_IP}:${E2E_PROXY_LISTEN}"
echo "[06] Windows will use proxy: $WIN_PROXY_URL" >&2

# Probe from Windows side that the proxy is reachable.
win_exec "powershell -NoProfile -Command \"Test-NetConnection -ComputerName ${MAC_IP} -Port ${E2E_PROXY_LISTEN} -InformationLevel Quiet\"" 2>&1 | \
    grep -q "True" \
    && _ok "Windows can reach Mac mini-proxy at ${MAC_IP}:${E2E_PROXY_LISTEN}" \
    || { _fail "Windows cannot reach Mac mini-proxy at ${MAC_IP}:${E2E_PROXY_LISTEN} — Mac firewall?"; exit 5; }

# Cleanup + ensure bundle dir.
win_exec "mkdir \"%USERPROFILE%\\Desktop\\ClawEnv\" 2>NUL & $WIN_CLAWCLI --json uninstall --name $NAME 2>NUL" >/dev/null 2>&1 || true

# Wrap cli_win with proxy-env prefix — the Windows ENV_PREFIX path will
# `set` these before clawcli runs in cmd.exe.
cli_win_with_proxy() {
    local old_prefix="$WIN_ENV_PREFIX"
    export WIN_ENV_PREFIX="set HTTP_PROXY=${WIN_PROXY_URL}&&set HTTPS_PROXY=${WIN_PROXY_URL}&&set NO_PROXY=localhost,127.0.0.1&&${old_prefix}"
    cli_win "$@"
    local rc=$?
    export WIN_ENV_PREFIX="$old_prefix"
    return $rc
}

echo ">> [1/9] Windows install native with proxy → $WIN_PROXY_URL" >&2
cli_win_with_proxy install --mode native --claw-type openclaw --version latest --name "$NAME" --port "$PORT"
expect_config_entry_win "$NAME"

echo ">> [2/9] start + gateway check" >&2
cli_win start "$NAME"
expect_http_200_win "http://127.0.0.1:${PORT}/health" 60

echo ">> [3/9] proxy diagnose reflects env-sourced proxy" >&2
win_exec "$WIN_CLAWCLI proxy diagnose" 2>&1 | \
    tee -a "$E2E_TEST_HOME/clawenv-e2e.log" | \
    grep -qE "Installer.*${MAC_IP}:${E2E_PROXY_LISTEN}" \
    && _ok "Installer scope picked up env proxy" \
    || _fail "proxy diagnose didn't reflect the injected proxy"

echo ">> [4/9] export" >&2
cli_win export "$NAME" --output "$WIN_BUNDLE"
expect_file_win "$WIN_BUNDLE"

echo ">> [5/9] uninstall" >&2
cli_win uninstall --name "$NAME"
expect_no_config_entry_win "$NAME"

echo ">> [6/9] import (proxy still on)" >&2
cli_win_with_proxy install --mode native --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --image "$WIN_BUNDLE"
expect_config_entry_win "$NAME"

echo ">> [7/9] start + curl" >&2
cli_win start "$NAME"
expect_http_200_win "http://127.0.0.1:${PORT}/health" 60

echo ">> [8/9] final uninstall" >&2
cli_win uninstall --name "$NAME"
expect_no_config_entry_win "$NAME"

echo ">> [9/9] cleanup bundle" >&2
win_exec "del \"$WIN_BUNDLE\" 2>NUL" >/dev/null || true

e2e_assert_summary
