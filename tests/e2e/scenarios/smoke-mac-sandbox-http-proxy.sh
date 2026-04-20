#!/bin/bash
# Smoke probe — macOS Lima sandbox, HTTP proxy (host OS).
# VM provision uses host proxy; in-VM apk+npm+git through it.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

[ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "macOS HTTPEnable=0 — no HTTP proxy configured"

export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"

NAME="probe-mac-sb-http"
PORT="11402"

cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> step prereq + create via HTTP $E2E_MAC_HTTP_PROXY" >&2
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --step prereq
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --step create
_ok "VM ready"

echo ">> probe net-check (apk+npm+git inside VM via HTTP proxy)" >&2
cli net-check --mode sandbox --name "$NAME" --probe apk,npm,git --proxy-url "$E2E_MAC_HTTP_PROXY"
_ok "sandbox net probes pass via HTTP proxy"

# `--step create` doesn't write a config.toml entry, so uninstall may
# report "not found" — tolerate. The VM is still removed by the
# uninstall flow's lima destroy step.
cli uninstall --name "$NAME" 2>/dev/null || true
_ok "cleanup done"
