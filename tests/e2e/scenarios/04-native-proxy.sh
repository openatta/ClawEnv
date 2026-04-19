#!/bin/bash
# Scenario 04: native install with proxy — mini_proxy relays to user's
# real local proxy. Tests the Scope::Installer priority chain + download
# helper's proxy env propagation (reqwest reads HTTPS_PROXY automatically).

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

NAME="e2e-nat-proxy"
PORT="10260"
BUNDLE_DIR="${E2E_BUNDLE_DIR:-$E2E_REAL_HOME/Desktop/ClawEnv}"
BUNDLE="$BUNDLE_DIR/${NAME}-$(date +%Y%m%d-%H%M%S).tar.gz"
mkdir -p "$BUNDLE_DIR"

e2e_assert_init

# Point clawcli at the mini-proxy (started by run.sh). This is the
# scripted equivalent of "Tauri GUI detected OS proxy and injected into
# child CLI env" — since we're bypassing Tauri, we inject directly.
export HTTPS_PROXY="http://127.0.0.1:${E2E_PROXY_LISTEN}"
export HTTP_PROXY="http://127.0.0.1:${E2E_PROXY_LISTEN}"
export NO_PROXY="localhost,127.0.0.1"

cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> [1/9] install native with proxy → $HTTPS_PROXY" >&2
cli install \
    --mode native \
    --claw-type openclaw \
    --version latest \
    --name "$NAME" \
    --port "$PORT"

expect_config_entry "$NAME"

echo ">> [2/9] start + gateway check" >&2
# Install/import auto-starts the gateway — skip redundant start.
expect_http_200 "http://127.0.0.1:${PORT}/health" 45

echo ">> [3/9] proxy diagnose confirms Scope::Installer used proxy" >&2
"$(e2e_cli_bin)" proxy diagnose 2>&1 | \
    tee -a "$E2E_TEST_HOME/clawenv-e2e.log" | \
    grep -qE "Installer.*127\\.0\\.0\\.1:${E2E_PROXY_LISTEN}" \
    && _ok "proxy diagnose reports env-sourced proxy" \
    || _fail "proxy diagnose didn't pick up our env proxy"

echo ">> [4/9] export native bundle" >&2
cli export "$NAME" --output "$BUNDLE"
expect_file "$BUNDLE"
expect_bundle_manifest "$BUNDLE"

echo ">> [5/9] uninstall" >&2
cli uninstall --name "$NAME"
expect_no_config_entry "$NAME"

echo ">> [6/9] import native bundle (proxy still on)" >&2
cli install \
    --mode native \
    --claw-type openclaw \
    --version latest \
    --name "$NAME" \
    --port "$PORT" \
    --image "$BUNDLE"

echo ">> [7/9] start + gateway check" >&2
# Install/import auto-starts the gateway — skip redundant start.
expect_http_200 "http://127.0.0.1:${PORT}/health" 45

echo ">> [8/9] final uninstall" >&2
cli uninstall --name "$NAME"
expect_no_config_entry "$NAME"

echo ">> [9/9] cleanup" >&2
rm -f "$BUNDLE"

e2e_assert_summary
