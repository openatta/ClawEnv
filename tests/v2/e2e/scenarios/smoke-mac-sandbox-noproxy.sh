#!/bin/bash
# Smoke probe — macOS Lima sandbox, no proxy.
# Provisions a Lima VM with openclaw installed and runs an in-VM
# net-check probe through the direct connection. Validates the full
# install pipeline (Lima boot + cloud-init + apk + npm + claw deploy)
# without any proxy env. ~8-10min wall.
#
# v2 note: v1 had `install --step prereq|create` to do VM-only
# provisioning. v2's pipeline runs end-to-end — slower but actually
# exercises the install path that ships to users.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

# Mac-only — Lima isn't a thing on Linux/Win.
case "$(uname -s)" in
    Darwin) : ;;
    *) _skip "macOS-only scenario (uname=$(uname -s))" ;;
esac

unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

e2e_preflight_noproxy

NAME="probe-mac-sb-noproxy"
PORT="11401"

cli instance destroy "$NAME" 2>/dev/null || true

echo ">> install (Lima VM + openclaw, no proxy)" >&2
cli install openclaw --backend lima --version latest --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
expect_limactl_running "$NAME"
_ok "VM ready + claw installed"

echo ">> probe net-check from inside VM (npm + github + nodejs.org)" >&2
cli net-check --mode sandbox "$NAME"
_ok "sandbox net probes pass with no proxy"

cli instance destroy "$NAME" 2>/dev/null || true
expect_no_config_entry "$NAME"
_ok "cleanup done"
