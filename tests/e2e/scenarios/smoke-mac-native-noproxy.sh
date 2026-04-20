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

# Network preflight — we're not testing the install pipeline's resilience
# to GFW (that's a separate concern). If the host can't reach github
# direct, skip rather than spend 30s probing a doomed connection.
if ! curl -sSf -m 5 --head https://registry.npmjs.org/ >/dev/null 2>&1; then
    _skip "registry.npmjs.org unreachable direct — no-proxy probe can't pass"
fi

echo ">> probe net-check (host+npm+git, no proxy)" >&2
cli net-check --mode native --probe host,npm,git --proxy-url ""
_ok "native net probes pass with no proxy"
