#!/bin/bash
# Smoke probe — export → destroy → import roundtrip on macOS Lima.
# Installs a minimal openclaw, exports it to a tarball, destroys, then
# re-imports from the same tarball and confirms the registry record
# comes back. Covers the distribution path no other scenario touches.
#
# Wall: ~10-12min (full install + export + import + verify).
#
# Why proxy is required: the install half pulls a fresh Lima base
# image + apk + openclaw npm package. Without a working proxy on a
# GFW-restricted network those downloads stall. If you're on a
# direct-reachable connection, set E2E_FORCE_NOPROXY=1 to bypass.

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

if [ "${E2E_FORCE_NOPROXY:-0}" = "1" ]; then
    unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
    e2e_preflight_noproxy
else
    [ -n "${E2E_MAC_HTTP_PROXY:-}" ] || _skip "roundtrip needs proxy (install pulls image+apk+npm); set E2E_FORCE_NOPROXY=1 to bypass"
    e2e_preflight_proxy "$E2E_MAC_HTTP_PROXY"
    export HTTP_PROXY="$E2E_MAC_HTTP_PROXY"
    export HTTPS_PROXY="$E2E_MAC_HTTP_PROXY"
fi

NAME="probe-mac-roundtrip"
PORT="11701"
EXPORT_DIR="${E2E_TEST_HOME:-/tmp}/clawenv-e2e-export"
mkdir -p "$EXPORT_DIR"
BUNDLE="$EXPORT_DIR/${NAME}.tar.gz"

cli instance destroy "$NAME" 2>/dev/null || true
rm -f "$BUNDLE"

# ---- Phase 1: install ----
echo ">> phase 1: install (Lima + openclaw)" >&2
cli install openclaw --backend lima --version latest --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "phase 1: installed"

# ---- Phase 2: export ----
echo ">> phase 2: export → $BUNDLE" >&2
cli export "$NAME" --output "$BUNDLE"
expect_file "$BUNDLE"
# Manifest sanity — bundle MUST have clawenv-bundle.toml at root and
# its schema_version must be exactly 1 (anything else means the writer
# bumped silently or the bundle is malformed).
manifest=$(tar -xzf "$BUNDLE" -O clawenv-bundle.toml 2>/dev/null || echo "")
if echo "$manifest" | grep -q '^schema_version = 1$'; then
    _ok "phase 2: bundle manifest valid (schema v1)"
else
    _fail "phase 2: bundle missing or has unexpected manifest"
    echo "  manifest excerpt:" >&2
    echo "$manifest" | head -10 >&2
    exit 1
fi

# ---- Phase 3: destroy (so import doesn't clash on name) ----
echo ">> phase 3: destroy" >&2
cli instance destroy "$NAME"
expect_no_config_entry "$NAME"
_ok "phase 3: destroyed"

# ---- Phase 4: re-import ----
echo ">> phase 4: re-import from $BUNDLE" >&2
cli import --file "$BUNDLE" --name "$NAME" --port "$PORT"
expect_config_entry "$NAME"
_ok "phase 4: re-imported"

# ---- Phase 5: bring VM back online ----
echo ">> phase 5: start re-imported VM" >&2
cli start "$NAME"
expect_limactl_running "$NAME"
_ok "phase 5: VM running post-import"

# Cleanup
cli instance destroy "$NAME" 2>/dev/null || true
rm -f "$BUNDLE"
_ok "cleanup done"
