#!/bin/bash
# Connectivity preflight shared by every smoke scenario.
#
# v2 contract (mirrors v1): network is the user's problem. If the
# baseline connection the scenario wants to exercise isn't actually
# reachable, we exit 2 (NOT the SKIP code 77 — a probe can't run, it's
# a hard fail from the test runner's point of view) with an English
# message telling the operator to fix their network before retrying.
#
# Lifted verbatim from v1 tests/e2e/lib/preflight.sh. v2 keeps the
# same canonical endpoint set (npm CDN + github + nodejs.org) because
# install pipelines reach exactly those three.
#
# Two modes:
#   e2e_preflight_noproxy
#     Probes one canonical endpoint direct (no HTTP_PROXY / HTTPS_PROXY
#     respected). Use from no-proxy smoke scripts.
#
#   e2e_preflight_proxy "$proxy_url"
#     Probes one canonical endpoint through the given proxy. Use from
#     http-proxy smoke scripts AFTER resolving the proxy URL.

_preflight_endpoints=(
    "https://registry.npmjs.org/"
    "https://api.github.com/"
    "https://nodejs.org/dist/"
)
# 15s per endpoint — aligns with cli/main.rs `run_native_probe`.
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

_preflight_run_all() {
    local mode="$1"
    local failures=""
    for url in "${_preflight_endpoints[@]}"; do
        local ok=1
        case "$mode" in
            direct)
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
