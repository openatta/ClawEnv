#!/bin/bash
# Smoke probe — Linux Podman sandbox, no proxy.
# Spins up a rootless Podman Alpine container with openclaw and runs
# the in-VM net-check probe. Validates Podman rootless + apk + npm +
# git + claw deploy without any proxy. ~5-8min wall.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

case "$(uname -s)" in
    Linux) : ;;
    *) _skip "Linux-only scenario (uname=$(uname -s))" ;;
esac

# Podman must be installed on the host. No auto-install — this is a
# smoke test, not a provisioner.
if ! command -v podman >/dev/null; then
    _skip "podman not installed on host — scenario requires rootless podman"
fi

unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

e2e_preflight_noproxy

NAME="probe-linux-podman-noproxy"
PORT="11601"

cli instance destroy "$NAME" 2>/dev/null || true

echo ">> install (Podman container + openclaw, no proxy)" >&2
cli install openclaw --backend podman --version latest --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "container ready + claw installed"

echo ">> probe net-check from inside container" >&2
cli net-check --mode sandbox "$NAME"
_ok "linux podman net probes pass with no proxy"

cli instance destroy "$NAME" 2>/dev/null || true
expect_no_config_entry "$NAME"
_ok "cleanup done"
