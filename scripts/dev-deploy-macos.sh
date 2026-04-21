#!/bin/bash
# Dev deploy: build BOTH ClawEnv (main) and ClawLite release bundles on
# macOS and copy all four artifacts (.app + .dmg for each) into a
# target directory so they can be compared side-by-side during debugging.
#
# Usage:
#   bash scripts/dev-deploy-macos.sh               # defaults to ~/Desktop/ClawEnv
#   bash scripts/dev-deploy-macos.sh /some/dir
set -euo pipefail

DEST="${1:-$HOME/Desktop/ClawEnv}"
mkdir -p "$DEST"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

export PATH="$HOME/.cargo/bin:$PATH"

echo "============================================"
echo "  Dev deploy — building ClawEnv + ClawLite"
echo "  Destination: $DEST"
echo "============================================"

# The bundle/dmg/ output dir is SHARED between main and lite builds — each
# `cargo tauri build` stage regenerates it, overwriting the other flavor's
# DMG. Deploy each flavor's artifacts IMMEDIATELY after its own build so
# nothing gets clobbered before it lands on the desktop.

deploy_app() {
    local src="$1"
    if [ -d "$src" ]; then
        local name
        name=$(basename "$src")
        rm -rf "$DEST/$name" 2>/dev/null || true
        # ditto handles running-app lock + extended attrs better than cp -R
        ditto "$src" "$DEST/$name"
        echo "  $name  $(du -sh "$DEST/$name" | awk '{print $1}')"
    else
        echo "  MISSING: $src"
    fi
}
deploy_dmg() {
    local src="$1"
    if [ -n "$src" ] && [ -f "$src" ]; then
        cp -f "$src" "$DEST/"
        local base
        base=$(basename "$src")
        echo "  $base  $(du -sh "$DEST/$base" | awk '{print $1}')"
    fi
}

# --- ClawEnv (main) + deploy immediately ---
echo ""
echo "--- Building ClawEnv (main) ---"
cargo tauri build
deploy_app "target/release/bundle/macos/ClawEnv.app"
deploy_dmg "$(find target/release/bundle/dmg -name 'ClawEnv_*.dmg' 2>/dev/null | head -1)"

# --- ClawLite + deploy immediately ---
echo ""
echo "--- Building ClawLite ---"
# Shares frontend+binary with ClawEnv; the config override only changes
# productName/identifier/window. `cd tauri` is required because Tauri CLI
# treats the --config file's parent dir as the Tauri project root (looks
# for Cargo.toml there), and lite/ doesn't have one.
( cd tauri && cargo tauri build --config ../lite/clawlite.tauri.conf.json )
deploy_app "target/release/bundle/macos/ClawLite.app"
deploy_dmg "$(find target/release/bundle/dmg -name 'ClawLite_*.dmg' 2>/dev/null | head -1)"

echo ""
echo "Deploy complete. Contents of $DEST:"
ls -1 "$DEST"
