#!/bin/bash
# Scenario 01: sandbox install → start → curl → export → delete
# → import → start → curl → delete.  No proxy.
#
# Assumptions: macOS host, Lima + dependencies already installed in
# user's real ~/.clawenv/bin (we reuse their limactl). User has
# internet access. No host proxy required.

set -eu

# Sourced libs are already loaded by run.sh; re-source defensively if
# the scenario is invoked directly.
if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

NAME="e2e-sb-noproxy"
PORT="10200"
BUNDLE_DIR="${E2E_BUNDLE_DIR:-$E2E_REAL_HOME/Desktop/ClawEnv}"
BUNDLE="$BUNDLE_DIR/${NAME}-$(date +%Y%m%d-%H%M%S).tar.gz"
mkdir -p "$BUNDLE_DIR"

e2e_assert_init

# Ensure no prior state leaks in (belt-and-braces; isolate.sh already
# gave us a fresh $HOME but guard anyway).
cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> [1/9] install sandbox instance '$NAME' on port $PORT" >&2
cli install \
    --mode sandbox \
    --claw-type openclaw \
    --version latest \
    --name "$NAME" \
    --port "$PORT"

expect_config_entry "$NAME"
expect_limactl_running "clawenv-"

echo ">> [2/9] start instance" >&2
cli start "$NAME"

echo ">> [3/9] curl gateway /health on port $PORT" >&2
expect_http_200 "http://127.0.0.1:${PORT}/health" 90

echo ">> [4/9] export bundle to $BUNDLE" >&2
cli export "$NAME" --output "$BUNDLE"
expect_file "$BUNDLE"
expect_bundle_manifest "$BUNDLE"

echo ">> [5/9] uninstall (VM + config gone)" >&2
cli uninstall --name "$NAME"
expect_no_limactl "clawenv-"
expect_no_config_entry "$NAME"

echo ">> [6/9] import bundle (re-install from tarball)" >&2
cli install \
    --mode sandbox \
    --claw-type openclaw \
    --version latest \
    --name "$NAME" \
    --port "$PORT" \
    --image "$BUNDLE"
expect_config_entry "$NAME"

echo ">> [7/9] start re-imported instance" >&2
cli start "$NAME"
expect_http_200 "http://127.0.0.1:${PORT}/health" 90

echo ">> [8/9] final uninstall" >&2
cli uninstall --name "$NAME"
expect_no_limactl "clawenv-"
expect_no_config_entry "$NAME"

echo ">> [9/9] cleanup bundle" >&2
rm -f "$BUNDLE"

e2e_assert_summary
