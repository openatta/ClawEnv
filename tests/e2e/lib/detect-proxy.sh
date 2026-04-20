#!/bin/bash
# Host proxy detection — per-platform, per-protocol.
#
# Exports three globals when a proxy of that type is configured on the
# host OS. Empty values mean "not configured" — scenarios that need
# that protocol should skip cleanly instead of erroring.
#
# Design: scenarios consume OS proxy as-is. No mini_proxy, no relay.
# If the host has HTTP but not SOCKS, SOCKS scenarios skip. This keeps
# the test surface honest — we're validating clawenv's proxy plumbing,
# not a synthetic relay that might hide bugs.

# macOS: scutil --proxy returns structured output with HTTPEnable,
# HTTPProxy, HTTPPort and SOCKSEnable, SOCKSProxy, SOCKSPort keys.
# HTTPSEnable rides the same fields via HTTPSPort; if both HTTP + HTTPS
# are set to the same server we expose just the HTTP one (npm et al.
# use HTTPS_PROXY=http://... for CONNECT-to-HTTPS anyway).
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

# Windows: IE/WinINET proxy config lives in the registry under
# HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings.
# ProxyEnable (DWORD) + ProxyServer (string, "host:port" or
# "http=host:port;https=host:port;..."). SOCKS isn't a first-class
# Windows OS setting — apps that want SOCKS typically bundle their own.
# We therefore only detect HTTP on Windows.
detect_win_http_proxy() {
    if [ -z "${WIN_USER:-}" ] || [ -z "${WIN_HOST:-}" ]; then return 0; fi
    local raw
    # Fetch both fields in one round-trip to avoid SSH latency.
    raw=$(ssh -o BatchMode=yes -o ConnectTimeout=5 "$WIN_USER@$WIN_HOST" \
        'powershell -NoProfile -Command "$s = Get-ItemProperty -Path \"HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\"; if ($s.ProxyEnable -eq 1) { Write-Output $s.ProxyServer } else { Write-Output \"\" }"' \
        2>/dev/null | tr -d '\r\n')
    if [ -z "$raw" ]; then return 0; fi
    # ProxyServer can be plain "host:port" or "http=host:port;https=..."
    # Extract the http= entry if present, else take the raw value.
    local pair
    if [[ "$raw" == *"="* ]]; then
        pair=$(echo "$raw" | tr ';' '\n' | awk -F= '$1 == "http" {print $2}')
    else
        pair="$raw"
    fi
    if [ -n "$pair" ]; then
        export E2E_WIN_HTTP_PROXY="http://${pair}"
    fi
}

# Print a one-line summary of what we detected. Call at the top of
# scenarios that depend on proxy state.
detect_proxy_summary() {
    echo "[detect-proxy] mac-http=${E2E_MAC_HTTP_PROXY:-<none>} mac-socks=${E2E_MAC_SOCKS_PROXY:-<none>} win-http=${E2E_WIN_HTTP_PROXY:-<none>}" >&2
}
