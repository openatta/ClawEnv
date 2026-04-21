#!/bin/bash
# Smoke probe — macOS Lima sandbox, no proxy.
# Spins up the smallest possible VM (apk + jq), then runs net-check
# against it. Validates VM provisioning + apk + npm + git in a fresh
# Alpine without any proxy. ~3-5min (VM boot + apk).

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

# Shared preflight — direct connection must work before we spin up
# a Lima VM that will try to apk/npm/git through the same path.
e2e_preflight_noproxy

NAME="probe-mac-sb-noproxy"
PORT="11401"

cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> step prereq + create (Lima VM with apk base packages)" >&2
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --step prereq
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --step create
_ok "VM ready"

echo ">> probe net-check (apk+npm+git inside VM, no proxy)" >&2
cli net-check --mode sandbox --name "$NAME" --probe apk,npm,git --proxy-url ""
_ok "sandbox net probes pass with no proxy"

cli uninstall --name "$NAME" 2>/dev/null || true
_ok "cleanup done"
