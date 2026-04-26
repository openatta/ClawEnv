#!/bin/bash
# Smoke probe — full install matrix on macOS via system HTTP proxy.
#
# Exercises every CLI install path users will actually hit:
#   1. native + openclaw   (host install, OpenClaw supports native)
#   2. lima   + openclaw   (sandbox install)
#   3. lima   + hermes     (sandbox install, the production claw)
#   4. native + hermes     (negative: Hermes does NOT support native;
#                           orchestrator must bail clean at validation)
#
# Each combo runs in its own sub-shell on the same isolated $HOME so
# we can re-use prewarm and detect regressions: a fix made for combo 2
# must keep combo 1 green.
#
# Wall budget per combo:
#   native + openclaw : ~3-5 min  (node + git + npm install)
#   lima   + openclaw : ~8-10 min (Lima boot + apk + npm install)
#   lima   + hermes   : ~10-15 min (Lima boot + apk + python + uv + git+pip)
#   native + hermes   : <5s (validation bail)
# Aggregate worst case: ~30 min.

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

# Honour $E2E_FORCE_NOPROXY=1 to run against direct egress, otherwise
# hard-require a system HTTP proxy. The user explicitly asked for the
# proxy variant so this is the default.
if [ "${E2E_FORCE_NOPROXY:-0}" = "1" ]; then
    unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
    e2e_preflight_noproxy
    PROXY_NOTE="(direct, no proxy)"
else
    [ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "macOS HTTPEnable=0 — no system HTTP proxy configured (set one in System Settings → Network → Proxies, or pass E2E_FORCE_NOPROXY=1)"
    e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"
    export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
    export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"
    PROXY_NOTE="(via $E2E_MAC_HTTP_PROXY)"
fi

# ──────────────────────────────────────────────────────────────
# Per-combo runner. Wraps the install + verify + cleanup so a
# failure in one combo doesn't cascade and all of them report.
#   combo_run <label> <expect:pass|expect:fail> <name> <port> -- <install args...>
# ──────────────────────────────────────────────────────────────
combo_run() {
    local label="$1"; shift
    local expect="$1"; shift   # expect:pass | expect:fail
    local name="$1"; shift
    local port="$1"; shift
    [ "$1" = "--" ] || { echo "combo_run: expected -- before install args"; return 2; }
    shift
    local install_args=("$@")

    echo ""
    echo "════════════════════════════════════════════════════════════════"
    echo "▶ combo: $label  $PROXY_NOTE"
    echo "  expect: $expect    name: $name    port: $port"
    echo "════════════════════════════════════════════════════════════════"

    cli instance destroy "$name" 2>/dev/null || true

    local rc=0
    cli install "${install_args[@]}" --name "$name" --port "$port" || rc=$?

    case "$expect" in
        expect:pass)
            if [ "$rc" -ne 0 ]; then
                _fail "combo $label: install failed (rc=$rc) but was expected to pass"
                return 1
            fi
            expect_config_entry "$name"
            # `clawcli status` must succeed for a passing install.
            cli status "$name" >/dev/null || { _fail "combo $label: status command failed post-install"; return 1; }
            _ok "combo $label: install + status OK"
            ;;
        expect:fail)
            if [ "$rc" -eq 0 ]; then
                _fail "combo $label: install succeeded (rc=0) but was expected to bail"
                return 1
            fi
            # Negative case must NOT have left a registry entry behind.
            expect_no_config_entry "$name"
            _ok "combo $label: bailed cleanly (rc=$rc) with no registry residue"
            ;;
        *)
            echo "combo_run: unknown expect mode: $expect" >&2
            return 2 ;;
    esac

    cli instance destroy "$name" 2>/dev/null || true
    return 0
}

# Order: cheapest → most expensive. A bug in the cheap cases surfaces
# in seconds; a bug in the expensive cases doesn't block faster signal.
OVERALL_RC=0
combo_run "native+hermes" expect:fail "matrix-native-hm" "12004" -- \
    hermes --backend native --version latest --autoinstall-deps \
    || OVERALL_RC=$?

combo_run "native+openclaw" expect:pass "matrix-native-oc" "12001" -- \
    openclaw --backend native --version latest --autoinstall-deps \
    || OVERALL_RC=$?

combo_run "lima+openclaw" expect:pass "matrix-lima-oc" "12002" -- \
    openclaw --backend lima --version latest \
    || OVERALL_RC=$?

combo_run "lima+hermes" expect:pass "matrix-lima-hm" "12003" -- \
    hermes --backend lima --version latest \
    || OVERALL_RC=$?

echo ""
e2e_assert_summary
exit "$OVERALL_RC"
