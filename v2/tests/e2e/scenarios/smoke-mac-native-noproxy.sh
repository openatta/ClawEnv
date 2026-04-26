#!/bin/bash
# Smoke probe — macOS Native, no proxy.
# Validates `clawcli net-check --mode host` against the direct
# connection. v2's host probe goes straight through reqwest and does
# not require a clawenv-native node/git toolchain (v1 had a
# has_node/has_git gate; v2 host probe is pure HTTP). ~30s wall.

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

unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

e2e_preflight_noproxy

echo ">> probe net-check (host, no proxy)" >&2
cli net-check --mode host
_ok "host net probes pass with no proxy"
