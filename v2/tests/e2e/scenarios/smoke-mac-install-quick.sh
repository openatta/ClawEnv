#!/bin/bash
# Quick subset of smoke-mac-install-matrix.sh — only the cheap combos
# (native+hermes negative + native+openclaw positive). Used to
# regression-test changes to install verb / assertions WITHOUT paying
# the 25min Lima provisioning cost.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

case "$(uname -s)" in
    Darwin) : ;;
    *) _skip "macOS-only scenario" ;;
esac

if [ "${E2E_FORCE_NOPROXY:-0}" = "1" ]; then
    unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
    e2e_preflight_noproxy
else
    [ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "no system HTTP proxy configured"
    e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"
    export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
    export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"
fi

# ────────────────────────────────────────────────
# combo 1 — native+hermes negative (instant bail)
# ────────────────────────────────────────────────
NAME1="quick-native-hm"
cli instance destroy "$NAME1" 2>/dev/null || true
echo ">> combo 1: native+hermes (expect: bail clean)" >&2
rc=0
cli install hermes --backend native --version latest --autoinstall-deps \
    --name "$NAME1" --port 12104 || rc=$?
[ "$rc" -ne 0 ] || { _fail "combo 1: native+hermes succeeded but should have bailed"; exit 1; }
expect_no_config_entry "$NAME1"
_ok "combo 1: bailed clean"

# ────────────────────────────────────────────────
# combo 2 — native+openclaw positive (~3-5min)
# ────────────────────────────────────────────────
NAME2="quick-native-oc"
cli instance destroy "$NAME2" 2>/dev/null || true
echo ">> combo 2: native+openclaw (expect: pass)" >&2
cli install openclaw --backend native --version latest --autoinstall-deps \
    --name "$NAME2" --port 12101
expect_config_entry "$NAME2"
cli status "$NAME2" >/dev/null
_ok "combo 2: install + status OK"

cli instance destroy "$NAME2" 2>/dev/null || true
expect_no_config_entry "$NAME2"
_ok "combo 2: cleanup verified"

e2e_assert_summary
