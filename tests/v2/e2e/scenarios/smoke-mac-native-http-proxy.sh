#!/bin/bash
# Smoke probe — macOS Native, HTTP proxy (host OS).
# Validates that v2's host-mode net-check honours HTTP_PROXY env vars.
# Pure host-side probe; no VM provisioning. ~30s wall.

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

e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"

export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"

echo ">> probe net-check (host via HTTP $E2E_MAC_HTTP_PROXY)" >&2
cli net-check --mode host
_ok "host net probes pass via HTTP proxy"
