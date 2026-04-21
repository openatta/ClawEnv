#!/bin/bash
# Smoke probe — Linux Podman sandbox, no proxy.
# Spins up a Podman Alpine container (apk + jq), then runs net-check
# against it. Validates Podman rootless + apk + npm + git in a fresh
# Alpine without any proxy. ~2-4min (image pull + apk).
#
# v0.3.0 note: requires a Linux host with Podman installed. When run
# from a Mac-driven CI pipeline this scenario should be SKIPPED by
# the parent shell (no Linux host available) — the skip is enforced
# here via a uname check. Wire this into run.sh once a Linux runner
# is available in the CI pool.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

# Hard gate: Linux-only scenario.
case "$(uname -s)" in
    Linux) : ;;
    *) _skip "Linux-only scenario (uname=$(uname -s))" ;;
esac

# Podman must be installed on the host. No attempt at auto-install —
# the scenario is a smoke test, not a provisioner.
if ! command -v podman >/dev/null; then
    _skip "podman not installed on host — scenario requires rootless podman"
fi

unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

# Preflight: host must reach upstream direct. Without this a podman
# `apk update` inside the container would stall on image pull / package
# fetch, costing minutes before clearly failing.
e2e_preflight_noproxy

NAME="probe-linux-podman-noproxy"
PORT="11601"

cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> step prereq + create (Podman + Alpine base packages)" >&2
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --step prereq
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT" --step create
_ok "Podman container ready"

echo ">> probe net-check (apk+npm+git inside container, no proxy)" >&2
cli net-check --mode sandbox --name "$NAME" --probe apk,npm,git --proxy-url ""
_ok "linux podman net probes pass with no proxy"

cli uninstall --name "$NAME" 2>/dev/null || true
_ok "cleanup done"
