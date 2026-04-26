#!/bin/bash
# Smoke probe — launch + gateway HTTP on macOS Lima.
#
# Tests the post-install runtime path: `clawcli launch` spawns the
# gateway daemon inside the VM via `nohup ... &`, probes the host
# port for readiness, and reports back. Then we curl the port from
# the host to confirm port-forwarding actually works.
#
# Wall: ~10-12min (full install + ~30s daemon startup + ~5s probe).
# Subset of P1-j coverage: install pipeline → launch → port probe.
#
# Why this matters: install pipeline can finish without the daemon
# being externally reachable (port-forward bug, daemon crash on first
# request, IPv6-only bind, etc.). This scenario proves the
# "user can hit the gateway" contract end-to-end.

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
    [ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "no system HTTP proxy configured (set one or pass E2E_FORCE_NOPROXY=1)"
    e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"
    export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
    export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"
fi

NAME="probe-mac-launch"
PORT="13001"

cli instance destroy "$NAME" 2>/dev/null || true

# ---- Phase 1: install ----
echo ">> phase 1: install openclaw" >&2
cli install openclaw --backend lima --version latest --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "phase 1: installed"

# ---- Phase 2: launch ----
# `launch` spawns gateway as a background daemon (nohup), probes the
# host-mapped port for up to 30s, and returns either ready_port=<P> on
# success or ready_port=null on timeout.
echo ">> phase 2: launch gateway daemon" >&2
launch_out=$("$(e2e_cli_bin)" --json launch "$NAME" 2>&1 | tee -a "${E2E_TEST_HOME:-/tmp}/clawenv-e2e.log")
ready_port=$(echo "$launch_out" | jq -r 'select(.type=="data") | .data.ready_port // empty' 2>/dev/null | tail -1)
if [ "$ready_port" = "$PORT" ]; then
    _ok "phase 2: launch reports ready_port=$PORT"
elif [ -z "$ready_port" ] || [ "$ready_port" = "null" ]; then
    _fail "phase 2: launch did not detect a ready port within timeout"
    echo "  launch output:" >&2
    echo "$launch_out" | tail -10 >&2
    cli instance destroy "$NAME" 2>/dev/null || true
    exit 1
else
    _fail "phase 2: launch reports unexpected ready_port=$ready_port (wanted $PORT)"
    cli instance destroy "$NAME" 2>/dev/null || true
    exit 1
fi

# ---- Phase 3: external HTTP probe ----
# launch's own probe runs inside the VM. We re-prove from the HOST
# via curl — catches port-forwarding bugs that the in-VM probe misses
# (e.g. Lima's portForwards stanza is wrong, or the VM bound to ::1
# only and the host can't reach v6-loopback).
echo ">> phase 3: external curl probe http://127.0.0.1:$PORT" >&2
expect_http_200 "http://127.0.0.1:$PORT" 30

# ---- Phase 4: stop VM, verify status ----
# Note: `cli stop` invokes `limactl stop` which terminates the qemu
# process. Lima's hostagent (a separate proxy process) sometimes
# lingers and keeps the host-side forwarded port LISTENING even after
# the VM is gone. That's an upstream Lima quirk — not a v2 bug — so
# this phase asserts only that the VM state flips to stopped, NOT
# that the port immediately closes. Phase 5 covers the full teardown.
echo ">> phase 4: stop VM" >&2
cli stop "$NAME"
state=$("$(e2e_cli_bin)" --json status "$NAME" 2>/dev/null \
    | jq -r 'select(.type=="data") | .data.vm.state // empty' | tail -1)
if [ "$state" = "stopped" ]; then
    _ok "phase 4: VM state = stopped"
else
    _fail "phase 4: expected vm.state=stopped, got '${state:-?}'"
    cli instance destroy "$NAME" 2>/dev/null || true
    exit 1
fi

# ---- Phase 5: destroy + verify port released ----
# `cli instance destroy` calls backend.destroy() which `limactl delete
# --force`s the VM and tears down the hostagent. Within ~10s the host
# port should release. Use --noproxy to bypass system HTTPS_PROXY
# (which would otherwise tunnel localhost back to itself, masking
# port closure with a 502 from the proxy).
echo ">> phase 5: destroy + verify port released" >&2
cli instance destroy "$NAME"
expect_no_config_entry "$NAME"
deadline=$(($(date +%s) + 15))
port_closed=0
while [ "$(date +%s)" -lt "$deadline" ]; do
    if ! curl -sSf --noproxy '*' -m 3 "http://127.0.0.1:$PORT" >/dev/null 2>&1; then
        port_closed=1
        break
    fi
    sleep 1
done
if [ "$port_closed" != 1 ]; then
    _fail "phase 5: gateway still responds 15s after destroy — hostagent leak"
    # Try to clean up by killing any stray limactl hostagents.
    pkill -f "limactl.*$NAME" 2>/dev/null || true
    exit 1
fi
_ok "phase 5: port released after full destroy"

e2e_assert_summary
