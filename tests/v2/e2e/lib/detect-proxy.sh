#!/bin/bash
# Host proxy detection — per-platform, per-protocol.
#
# Exports E2E_MAC_HTTP_PROXY / E2E_MAC_SOCKS_PROXY when a proxy of that
# type is configured on the host OS. Empty values mean "not configured"
# — scenarios that need that protocol should skip cleanly instead of
# erroring.
#
# Lifted from v1 tests/e2e/lib/detect-proxy.sh. v2 drops Windows
# detection (the win-* scenarios are deferred until the v2 wsl-rsync
# infra lands).

detect_mac_http_proxy() {
    if ! command -v scutil >/dev/null; then return 0; fi
    local out
    out=$(scutil --proxy 2>/dev/null)
    local enabled host port
    enabled=$(echo "$out" | awk '/HTTPEnable :/ {print $3}')
    host=$(echo "$out" | awk '/HTTPProxy :/ {print $3}')
    port=$(echo "$out" | awk '/HTTPPort :/ {print $3}')
    if [ "$enabled" = "1" ] && [ -n "$host" ] && [ -n "$port" ]; then
        export E2E_MAC_HTTP_PROXY="http://${host}:${port}"
    fi
}

detect_mac_socks_proxy() {
    if ! command -v scutil >/dev/null; then return 0; fi
    local out enabled host port
    out=$(scutil --proxy 2>/dev/null)
    enabled=$(echo "$out" | awk '/SOCKSEnable :/ {print $3}')
    host=$(echo "$out" | awk '/SOCKSProxy :/ {print $3}')
    port=$(echo "$out" | awk '/SOCKSPort :/ {print $3}')
    if [ "$enabled" = "1" ] && [ -n "$host" ] && [ -n "$port" ]; then
        export E2E_MAC_SOCKS_PROXY="socks5://${host}:${port}"
    fi
}

detect_proxy_summary() {
    echo "[detect-proxy] mac-http=${E2E_MAC_HTTP_PROXY:-<none>} mac-socks=${E2E_MAC_SOCKS_PROXY:-<none>}" >&2
}
