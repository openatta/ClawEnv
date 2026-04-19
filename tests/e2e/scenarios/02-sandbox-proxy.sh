#!/bin/bash
# Scenario 02: sandbox install with proxy — mini_proxy relays to user's
# real local proxy. Same flow as 01 but with HTTPS_PROXY set on the
# clawcli subprocess env pointing at our listen port.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

NAME="e2e-sb-proxy"
PORT="10220"
BUNDLE_DIR="${E2E_BUNDLE_DIR:-$E2E_REAL_HOME/Desktop/ClawEnv}"
BUNDLE="$BUNDLE_DIR/${NAME}-$(date +%Y%m%d-%H%M%S).tar.gz"
mkdir -p "$BUNDLE_DIR"

e2e_assert_init

# run.sh has already started mini-proxy if this scenario is included.
# E2E_PROXY_LISTEN is exported there.
export HTTPS_PROXY="http://127.0.0.1:${E2E_PROXY_LISTEN}"
export HTTP_PROXY="http://127.0.0.1:${E2E_PROXY_LISTEN}"
export NO_PROXY="localhost,127.0.0.1"

cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> [1/9] install sandbox with proxy → $HTTPS_PROXY" >&2
cli install \
    --mode sandbox \
    --claw-type openclaw \
    --version latest \
    --name "$NAME" \
    --port "$PORT"

expect_config_entry "$NAME"
expect_limactl_running "clawenv-"

echo ">> [2/9] start + gateway check" >&2
cli start "$NAME"
expect_http_200 "http://127.0.0.1:${PORT}/health" 90

echo ">> [3/9] verify proxy diagnose output has proxy applied" >&2
"$(e2e_cli_bin)" proxy diagnose --instance "$NAME" 2>&1 | \
    tee -a "$E2E_TEST_HOME/clawenv-e2e.log" | \
    grep -qE "127\\.0\\.0\\.1:${E2E_PROXY_LISTEN}|host\\.lima\\.internal:${E2E_PROXY_LISTEN}" \
    && _ok "proxy diagnose reports expected URL" \
    || _fail "proxy diagnose didn't show our mini-proxy"

echo ">> [4/9] export" >&2
cli export "$NAME" --output "$BUNDLE"
expect_file "$BUNDLE"
expect_bundle_manifest "$BUNDLE"

echo ">> [5/9] uninstall" >&2
cli uninstall --name "$NAME"
expect_no_limactl "clawenv-"

echo ">> [6/9] import via bundle (proxy still on)" >&2
cli install \
    --mode sandbox \
    --claw-type openclaw \
    --version latest \
    --name "$NAME" \
    --port "$PORT" \
    --image "$BUNDLE"

echo ">> [7/9] start + gateway check" >&2
cli start "$NAME"
expect_http_200 "http://127.0.0.1:${PORT}/health" 90

echo ">> [8/9] final uninstall" >&2
cli uninstall --name "$NAME"
expect_no_limactl "clawenv-"

echo ">> [9/9] cleanup" >&2
rm -f "$BUNDLE"

e2e_assert_summary
