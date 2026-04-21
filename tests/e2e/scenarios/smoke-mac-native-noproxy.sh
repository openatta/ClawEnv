#!/bin/bash
# Smoke probe — macOS Native, no proxy.
# Runs `clawcli net-check --mode native --probe host,npm,git` against the
# direct connection. Validates reqwest + npm + git can reach upstream
# without any proxy configured. ~30s wall.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

# Shared preflight: if direct connection is broken, fail fast (exit 2)
# rather than burning 30s on a doomed net-check probe. v0.3.0 contract —
# networking is the operator's problem, not the test harness's.
e2e_preflight_noproxy

NAME="probe-mn0"

cli uninstall --name "$NAME" 2>/dev/null || true

# Install ClawEnv-native prereq (node + git into ~/.clawenv/node/ and
# ~/.clawenv/git/). Without this step the subsequent net-check probe
# would fail the has_node/has_git gate added in run_native_probe —
# which is the honest behaviour: smoke tests MUST exercise ClawEnv's
# own toolchain, not the host's system node/git. Prewarm may have
# already seeded these from the real $HOME; prereq step is idempotent.
echo ">> step prereq (install clawenv-native node + git, no proxy)" >&2
cli install --mode native --claw-type openclaw --version latest \
    --name "$NAME" --port 11403 --step prereq
_ok "clawenv-native prereq ready"

echo ">> probe net-check (host+npm+git, no proxy)" >&2
cli net-check --mode native --probe host,npm,git --proxy-url ""
_ok "native net probes pass with no proxy"

cli uninstall --name "$NAME" 2>/dev/null || true
