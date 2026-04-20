#!/bin/bash
# Smoke probe — Windows Native, no proxy.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

source "$E2E_REPO_ROOT/tests/e2e/lib/win-remote.sh"
e2e_win_load_env || exit 3

e2e_assert_init

# Strip any proxy env from the Windows clawcli child process.
echo ">> probe net-check on Windows (host+npm+git, no proxy)" >&2
cli_win "net-check --mode native --probe host,npm,git --proxy-url \"\""
_ok "win-native net probes pass with no proxy"
