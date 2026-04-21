#!/bin/bash
# Smoke probe — Linux Podman sandbox, HTTP proxy.
# Builds Podman base image through the user's HTTP proxy, then runs
# net-check probes that exercise apk + npm + git through the same
# proxy via host.containers.internal.

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

if ! command -v podman >/dev/null; then
    _skip "podman not installed on host"
fi

[ -n "${E2E_LINUX_HTTP_PROXY:-}" ] || _skip "E2E_LINUX_HTTP_PROXY not set (export it to http://host:port to enable)"

e2e_preflight_proxy "$E2E_LINUX_HTTP_PROXY"

export HTTP_PROXY="$E2E_LINUX_HTTP_PROXY"
export HTTPS_PROXY="$E2E_LINUX_HTTP_PROXY"

NAME="probe-linux-podman-http"
PORT="11602"

cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> step prereq + create via HTTP $E2E_LINUX_HTTP_PROXY" >&2
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --step prereq
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --step create
_ok "Podman container ready (via proxy)"

echo ">> probe net-check (apk+npm+git inside container via HTTP proxy)" >&2
cli net-check --mode sandbox --name "$NAME" --probe apk,npm,git --proxy-url "$E2E_LINUX_HTTP_PROXY"
_ok "linux podman net probes pass via HTTP proxy"

cli uninstall --name "$NAME" 2>/dev/null || true
_ok "cleanup done"
