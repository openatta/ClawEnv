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

# Preflight runs ON THE WINDOWS VM via SSH — the subsequent clawcli.exe
# probes run there, so that's the network we actually need to verify.
# A Mac-side preflight would have been a different egress (different NIC,
# maybe different GFW reach) and produced false-positives/negatives.
e2e_preflight_noproxy_on_win

NAME="probe-wn0"

cli_win "uninstall --name \"$NAME\"" 2>/dev/null || true

# Install ClawEnv-native toolchain on Windows (node.exe + git.exe under
# %CLAWENV_HOME%\node\ and ...\git\). WIN_ENV_PREFIX intentionally does
# NOT include C:\Program Files\nodejs or \Git\cmd — the net-check probe
# must hit ClawEnv's own node/git, not the system ones. This step is
# what puts them there.
echo ">> step prereq on Windows (install clawenv-native node + git, no proxy)" >&2
cli_win "install --mode native --claw-type openclaw --version latest --name \"$NAME\" --port 11503 --step prereq"
_ok "clawenv-native prereq ready (Windows)"

echo ">> probe net-check on Windows (host+npm+git, no proxy)" >&2
cli_win "net-check --mode native --probe host,npm,git --proxy-url \"\""
_ok "win-native net probes pass with no proxy"

cli_win "uninstall --name \"$NAME\"" 2>/dev/null || true
