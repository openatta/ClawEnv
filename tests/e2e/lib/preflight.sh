#!/bin/bash
# Connectivity preflight shared by every smoke scenario.
#
# v0.3.0 contract: network is the user's problem. If the baseline
# connection the scenario wants to exercise isn't actually reachable,
# we exit 2 (*not* the SKIP code 77 — a probe can't run, it's a hard
# fail from the *test runner's* point of view) with an English message
# telling the operator to fix their network before retrying. CLI output
# is English only per project i18n policy; bilingual UX is reserved
# for the GUI installer.
#
# Two modes:
#   e2e_preflight_noproxy
#     Probes one canonical endpoint direct (no HTTP_PROXY / HTTPS_PROXY
#     respected). Use from no-proxy smoke scripts.
#
#   e2e_preflight_proxy "$proxy_url"
#     Probes one canonical endpoint through the given proxy. Use from
#     http-proxy smoke scripts AFTER resolving the proxy URL. Empty
#     argument falls through to noproxy mode.
#
# Both helpers exit 2 on failure, never return an error code to the
# caller (keeps scenarios with `set -e` simple — they don't need to
# guard the preflight call).

# All three canonical endpoints probed by every preflight call. Each
# covers a distinct failure mode: npm (Cloudflare CDN — usually works
# even on restricted networks), github (the classic restricted target),
# nodejs.org (dist downloads). A truly usable network must reach all
# three — probing npm alone gave false "ready" verdicts because CDNs
# mask the fact that github/nodejs.org are blocked.
_preflight_endpoints=(
    "https://registry.npmjs.org/"
    "https://api.github.com/"
    "https://nodejs.org/dist/"
)
# 15s per endpoint — aligns with cli/src/main.rs `run_native_probe`
# host-probe reqwest builder.
_preflight_timeout=15

_preflight_fail() {
    local failures="$1"
    echo "" >&2
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" >&2
    echo "✗ preflight: one or more endpoints unreachable" >&2
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━" >&2
    echo "" >&2
    echo "Network unreachable — fix network before running this test." >&2
    echo "" >&2
    echo "Failed endpoints:" >&2
    printf '%s\n' "$failures" >&2
    echo "" >&2
    echo "Timeout per endpoint: ${_preflight_timeout}s" >&2
    echo "" >&2
    echo "Hint:" >&2
    echo "  - If you need a proxy to reach npm/github/nodejs.org," >&2
    echo "    run the http-proxy smoke variant instead." >&2
    exit 2
}

# Probe every endpoint in `_preflight_endpoints`. All must succeed.
# $1 is one of "direct" / "proxy:<url>" — controls curl args.
_preflight_run_all() {
    local mode="$1"
    local failures=""
    for url in "${_preflight_endpoints[@]}"; do
        local ok=1
        case "$mode" in
            direct)
                # macOS's stock curl reads OS-level proxy config via
                # CFNetwork even when HTTP_PROXY env vars are unset — so
                # `env -u HTTP_PROXY` alone is insufficient. --noproxy '*'
                # forces a true direct probe.
                env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
                    -u http_proxy -u https_proxy -u all_proxy \
                  curl -sSf -m "${_preflight_timeout}" --head \
                      --noproxy '*' "$url" >/dev/null 2>&1 || ok=0 ;;
            proxy:*)
                local proxy="${mode#proxy:}"
                curl -sSf -m "${_preflight_timeout}" --head \
                     --proxy "$proxy" "$url" >/dev/null 2>&1 || ok=0 ;;
        esac
        if [ "$ok" = 1 ]; then
            echo "   ok  $url" >&2
        else
            echo "   FAIL $url" >&2
            failures+="  - ${url}"$'\n'
        fi
    done
    if [ -n "$failures" ]; then
        _preflight_fail "$failures"
    fi
}

e2e_preflight_noproxy() {
    echo ">> preflight: connectivity check (no proxy) across ${#_preflight_endpoints[@]} endpoints" >&2
    _preflight_run_all "direct"
    echo "   all endpoints reachable (direct)" >&2
}

e2e_preflight_proxy() {
    local proxy="$1"
    if [ -z "$proxy" ]; then
        e2e_preflight_noproxy
        return
    fi
    echo ">> preflight: connectivity check (via ${proxy}) across ${#_preflight_endpoints[@]} endpoints" >&2
    _preflight_run_all "proxy:${proxy}"
    echo "   all endpoints reachable (via proxy)" >&2
}

# ── Windows-side preflights ────────────────────────────────────────────
#
# Windows smoke scenarios exercise the Windows network stack, so the
# preflight must dial from Windows itself — not from the driving Mac.
# Critical for loopback proxies: on the Windows VM, 127.0.0.1:10808 is
# the local ClashX-style daemon; from Mac the same URL would hit Mac's
# loopback (a different proxy or nothing), giving a false negative.
#
# Requires WIN_HOST / WIN_USER already loaded by e2e_win_load_env.

_preflight_run_all_on_win() {
    local mode="$1"
    local failures=""
    for url in "${_preflight_endpoints[@]}"; do
        local cmd
        case "$mode" in
            direct)
                cmd="curl.exe -sSf -m ${_preflight_timeout} --head --noproxy * ${url}" ;;
            proxy:*)
                local proxy="${mode#proxy:}"
                cmd="curl.exe -sSf -m ${_preflight_timeout} --head --proxy ${proxy} ${url}" ;;
        esac
        if ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" "$cmd" >/dev/null 2>&1; then
            echo "   ok  $url" >&2
        else
            echo "   FAIL $url" >&2
            failures+="  - ${url}"$'\n'
        fi
    done
    if [ -n "$failures" ]; then
        _preflight_fail "$failures"
    fi
}

e2e_preflight_noproxy_on_win() {
    echo ">> preflight [win]: connectivity check (no proxy) across ${#_preflight_endpoints[@]} endpoints" >&2
    _preflight_run_all_on_win "direct"
    echo "   all endpoints reachable (win direct)" >&2
}

e2e_preflight_proxy_on_win() {
    local proxy="$1"
    if [ -z "$proxy" ]; then
        e2e_preflight_noproxy_on_win
        return
    fi
    echo ">> preflight [win]: connectivity check (via ${proxy}) across ${#_preflight_endpoints[@]} endpoints" >&2
    _preflight_run_all_on_win "proxy:${proxy}"
    echo "   all endpoints reachable (win via proxy)" >&2
}
