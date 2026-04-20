#!/bin/bash
# Smoke probe — macOS Native, HTTP proxy (host OS).
# Validates reqwest + npm + git all honour HTTP_PROXY when set.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

[ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "macOS HTTPEnable=0 — no HTTP proxy configured"

export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"

echo ">> probe net-check (host+npm+git via HTTP $E2E_MAC_HTTP_PROXY)" >&2
cli net-check --mode native --probe host,npm,git --proxy-url "$E2E_MAC_HTTP_PROXY"
_ok "native net probes pass via HTTP proxy"
