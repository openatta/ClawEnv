#!/bin/bash
# Smoke probe — export + re-import roundtrip on macOS.
# Installs a minimal openclaw instance, exports it to a tarball,
# uninstalls, then re-imports from the same tarball and confirms it
# starts. Covers the distribution path (`clawcli export` /
# `clawcli import`) that no other scenario touches.
#
# Wall clock: ~6-8 min (full install + export + import). Runs under
# the `sandbox-http-proxy` conditions because a real install cycle
# needs apk / npm to reach upstream; add a `-noproxy` variant once
# the current mac's network can direct-reach github/npm reliably.

set -eu

if [ -z "${E2E_REPO_ROOT:-}" ]; then
    echo "This scenario must be launched via run.sh" >&2
    exit 2
fi

e2e_assert_init

[ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "macOS HTTPEnable=0 — roundtrip uses proxied install"

e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"

export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"

NAME="probe-mac-roundtrip"
PORT="11701"
EXPORT_DIR="${E2E_TEST_HOME:-/tmp}/clawenv-e2e-export"
mkdir -p "$EXPORT_DIR"
BUNDLE="$EXPORT_DIR/${NAME}.tar.gz"

cli uninstall --name "$NAME" 2>/dev/null || true
rm -f "$BUNDLE"

# ---- Phase 1: install ----
echo ">> phase 1: install (sandbox, via proxy)" >&2
cli install --mode sandbox --claw-type openclaw --version latest --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "phase 1: installed"

# ---- Phase 2: export ----
echo ">> phase 2: export to $BUNDLE" >&2
cli export --name "$NAME" --output "$BUNDLE"
expect_bundle_manifest "$BUNDLE"
_ok "phase 2: exported"

# ---- Phase 3: uninstall (so the import doesn't clash on name/port) ----
echo ">> phase 3: uninstall" >&2
cli uninstall --name "$NAME"
expect_no_config_entry "$NAME"
_ok "phase 3: uninstalled"

# ---- Phase 4: re-import ----
echo ">> phase 4: re-import from $BUNDLE" >&2
cli import --file "$BUNDLE" --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "phase 4: re-imported"

# ---- Phase 5: start and probe gateway ----
echo ">> phase 5: start and probe gateway" >&2
cli start --name "$NAME"
expect_http_200 "http://127.0.0.1:${PORT}" 60
_ok "phase 5: gateway reachable post-import"

# Cleanup
cli uninstall --name "$NAME" 2>/dev/null || true
rm -f "$BUNDLE"
_ok "cleanup done"
