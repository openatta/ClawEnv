#!/bin/bash
# Scenario 03: native install → start → curl → export → delete
# → import → start → curl → delete. No proxy.
#
# Native mode uses our own bundled Node.js + git (see
# install_native/{macos,windows}.rs) — no Lima VM, no apk.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

NAME="e2e-nat-noproxy"
PORT="10240"
BUNDLE_DIR="${E2E_BUNDLE_DIR:-$E2E_REAL_HOME/Desktop/ClawEnv}"
BUNDLE="$BUNDLE_DIR/${NAME}-$(date +%Y%m%d-%H%M%S).tar.gz"
mkdir -p "$BUNDLE_DIR"

e2e_assert_init

# Ensure no state leaks.
cli uninstall --name "$NAME" 2>/dev/null || true

echo ">> [1/9] install native instance '$NAME' on port $PORT" >&2
cli install \
    --mode native \
    --claw-type openclaw \
    --version latest \
    --name "$NAME" \
    --port "$PORT"

expect_config_entry "$NAME"
# Native has no Lima VM — skip the limactl assertion.

echo ">> [2/9] verify sandbox_id=='native'" >&2
grep -A10 "^name = \"$NAME\"$" "$E2E_TEST_HOME/.clawenv/config.toml" | \
    grep -q 'sandbox_type = "native"' \
    && _ok "sandbox_type correctly set to native" \
    || _fail "sandbox_type not native in config"

echo ">> [3/9] start + gateway check" >&2
cli start "$NAME"
expect_http_200 "http://127.0.0.1:${PORT}/health" 45

echo ">> [4/9] export native bundle" >&2
cli export "$NAME" --output "$BUNDLE"
expect_file "$BUNDLE"
expect_bundle_manifest "$BUNDLE"

echo ">> [5/9] uninstall" >&2
cli uninstall --name "$NAME"
expect_no_config_entry "$NAME"

echo ">> [6/9] import native bundle" >&2
cli install \
    --mode native \
    --claw-type openclaw \
    --version latest \
    --name "$NAME" \
    --port "$PORT" \
    --image "$BUNDLE"
expect_config_entry "$NAME"

echo ">> [7/9] start re-imported instance" >&2
cli start "$NAME"
expect_http_200 "http://127.0.0.1:${PORT}/health" 45

echo ">> [8/9] final uninstall" >&2
cli uninstall --name "$NAME"
expect_no_config_entry "$NAME"

echo ">> [9/9] cleanup" >&2
rm -f "$BUNDLE"

e2e_assert_summary
