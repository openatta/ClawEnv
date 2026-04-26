#!/bin/bash
# Smoke probe — macOS Lima sandbox, HTTP proxy (host OS).
# Provision Lima + install openclaw with host-OS HTTP proxy active;
# in-VM apk + npm + git all routed through it. ~8-10min wall.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

case "$(uname -s)" in
    Darwin) : ;;
    *) _skip "macOS-only scenario (uname=$(uname -s))" ;;
esac

[ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "macOS HTTPEnable=0 — no HTTP proxy configured"

# Verify the proxy works before provisioning a 200MB VM through it.
e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"

export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"

NAME="probe-mac-sb-http"
PORT="11402"

cli instance destroy "$NAME" 2>/dev/null || true

echo ">> install via HTTP $E2E_MAC_HTTP_PROXY" >&2
cli install openclaw --backend lima --version latest --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
expect_limactl_running "$NAME"
_ok "VM ready + claw installed via proxy"

echo ">> probe net-check from inside VM (via HTTP proxy)" >&2
cli net-check --mode sandbox "$NAME"
_ok "sandbox net probes pass via HTTP proxy"

cli instance destroy "$NAME" 2>/dev/null || true
_ok "cleanup done"
