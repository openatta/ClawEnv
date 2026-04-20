#!/bin/bash
# Scenario 05: native install on Windows ARM64 via SSH. No proxy.
#
# Runs the same install → start → curl → export → delete → import →
# start → curl → delete cycle, but every operation happens ON the
# Windows box (via SSH). clawcli.exe must be pre-built at:
#   $WIN_PROJECT/target/release/clawcli.exe
#
# Assumes `.env` has WIN_HOST/WIN_USER/WIN_PROJECT set and the Windows
# box can SSH in passwordless (key auth) — see docs/15-cross-dev-windows.md.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

# win-remote.sh provides cli_win(), expect_http_200_win(),
# expect_file_win(), expect_config_entry_win().
source "$E2E_REPO_ROOT/tests/e2e/lib/win-remote.sh"
e2e_win_load_env || exit 3

NAME="e2e-win-nat-noproxy"
PORT="10280"
# Bundle lives on the Windows box — the user mirrors the ~/Desktop/ClawEnv
# convention via %USERPROFILE%\Desktop\ClawEnv\.
WIN_BUNDLE="%USERPROFILE%\\Desktop\\ClawEnv\\${NAME}-$(date +%Y%m%d-%H%M%S).tar.gz"

e2e_assert_init

# Ensure bundle dir + clean slate. Windows only allows ONE native install
# at a time (core/src/manager/install_native/mod.rs enforces this), so
# any pre-existing native must go — even if it's called something other
# than our test name. Parse the list, uninstall each. The test VM is
# expected to be dedicated to E2E.
win_exec "mkdir \"%USERPROFILE%\\Desktop\\ClawEnv\" 2>NUL" >/dev/null 2>&1 || true
existing=$(win_exec "$WIN_CLAWCLI --json list" 2>/dev/null | \
    tr -d '\r' | grep '^{' | jq -r 'select(.type=="data") | .data.instances[]? | select(.sandbox_type=="Native") | .name' 2>/dev/null)
for prev in $existing; do
    echo "[05] pre-existing native instance '$prev' — uninstalling" >&2
    win_exec "$WIN_CLAWCLI --json uninstall --name $prev" >/dev/null 2>&1 || true
done

echo ">> [1/9] Windows install native '$NAME' on port $PORT" >&2
cli_win install --mode native --claw-type openclaw --version latest --name "$NAME" --port "$PORT"
expect_config_entry_win "$NAME"

echo ">> [2/9] curl gateway /health — install already auto-started it" >&2
# Install/import auto-starts the gateway — skip redundant start.
expect_http_200_win "http://127.0.0.1:${PORT}/health" 60

echo ">> [4/9] export native bundle" >&2
cli_win export "$NAME" --output "$WIN_BUNDLE"
expect_file_win "$WIN_BUNDLE"

echo ">> [5/9] uninstall" >&2
cli_win uninstall --name "$NAME"
expect_no_config_entry_win "$NAME"

echo ">> [6/9] import bundle" >&2
cli_win install --mode native --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --image "$WIN_BUNDLE"
expect_config_entry_win "$NAME"

echo ">> [7/9] start re-imported + curl" >&2
# Install/import auto-starts the gateway — skip redundant start.
expect_http_200_win "http://127.0.0.1:${PORT}/health" 60

echo ">> [8/9] final uninstall" >&2
cli_win uninstall --name "$NAME"
expect_no_config_entry_win "$NAME"

echo ">> [9/9] cleanup bundle on Windows" >&2
win_exec "del \"$WIN_BUNDLE\" 2>NUL" >/dev/null || true

e2e_assert_summary
