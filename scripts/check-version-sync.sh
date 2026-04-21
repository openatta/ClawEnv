#!/usr/bin/env bash
# Fail CI if the version strings across the build configuration are out of
# sync. We shipped v0.2.5 with Cargo at 0.1.0 — the BundleManifest was
# stamping "0.1.0" into every exported archive — because nothing enforced
# the invariant. ClawEnv + ClawLite share the same Rust binary and frontend
# bundle; the only per-flavor file is `lite/clawlite.tauri.conf.json`
# which overrides productName/identifier/version.
#
# Exits 0 on agreement, 1 otherwise. Prints a table either way so CI logs
# show what the values actually are.

set -euo pipefail

cd "$(dirname "$0")/.."

extract_toml() {
    # Read the first `version = "..."` line under a [package] table.
    awk '/^\[package\]/{p=1} p && /^version/{gsub(/[" ]/,"",$3); print $3; exit}' "$1"
}

extract_json() {
    # Naive but sufficient: grab the first top-level `"version": "..."`.
    grep -m1 '"version":' "$1" | sed -E 's/.*"version":[[:space:]]*"([^"]+)".*/\1/'
}

CORE=$(extract_toml core/Cargo.toml)
CLI=$(extract_toml cli/Cargo.toml)
TAURI=$(extract_toml tauri/Cargo.toml)
TAURI_CONF=$(extract_json tauri/tauri.conf.json)
LITE_CONF=$(extract_json lite/clawlite.tauri.conf.json)
PKG=$(extract_json package.json)

printf '%-32s %s\n' \
    "core/Cargo.toml:"               "$CORE" \
    "cli/Cargo.toml:"                "$CLI" \
    "tauri/Cargo.toml:"              "$TAURI" \
    "tauri/tauri.conf.json:"         "$TAURI_CONF" \
    "lite/clawlite.tauri.conf.json:" "$LITE_CONF" \
    "package.json:"                  "$PKG"

# Use the first as the reference — if any differs, bail with a pointer to
# this script so whoever's cutting the release knows to sync all six.
REF="$CORE"
for v in "$CLI" "$TAURI" "$TAURI_CONF" "$LITE_CONF" "$PKG"; do
    if [[ "$v" != "$REF" ]]; then
        echo ""
        echo "ERROR: version drift detected." >&2
        echo "All six files must declare the same version. Update them and" >&2
        echo "re-run. Entry point: core/Cargo.toml is the reference." >&2
        exit 1
    fi
done

echo ""
echo "OK: all six version strings agree ($REF)."
