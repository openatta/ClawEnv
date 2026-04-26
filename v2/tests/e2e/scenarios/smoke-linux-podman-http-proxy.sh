#!/bin/bash
# Smoke probe — Linux Podman sandbox, HTTP proxy (host env).
# Provision Podman + install openclaw with HTTP_PROXY exported; the
# install pipeline propagates the proxy into the container's apk +
# npm + git. ~5-8min wall.

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

# Linux has no scutil; rely on whatever HTTP_PROXY the operator has
# already exported. Skip cleanly if absent.
[ -n "${HTTP_PROXY:-}${http_proxy:-}" ] || _skip "no HTTP_PROXY env set — export one to run"
PROXY="${HTTP_PROXY:-${http_proxy}}"

e2e_preflight_proxy "$PROXY"

export HTTP_PROXY="$PROXY"
export HTTPS_PROXY="$PROXY"

NAME="probe-linux-podman-http"
PORT="11602"

cli instance destroy "$NAME" 2>/dev/null || true

echo ">> install via HTTP $PROXY" >&2
cli install openclaw --backend podman --version latest --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "container ready + claw installed via proxy"

echo ">> probe net-check from inside container (via HTTP proxy)" >&2
cli net-check --mode sandbox "$NAME"
_ok "podman net probes pass via HTTP proxy"

cli instance destroy "$NAME" 2>/dev/null || true
_ok "cleanup done"
