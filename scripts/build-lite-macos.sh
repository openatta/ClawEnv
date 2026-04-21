#!/bin/bash
# ClawLite — macOS Build Script
#
# ClawLite shares the SAME frontend bundle and SAME Rust binary as
# ClawEnv. The only difference is the Tauri config override in
# `lite/clawlite.tauri.conf.json` which changes productName, identifier,
# version, and window dimensions. At runtime, `src/App.tsx` detects the
# app name and swaps the install component (`LiteInstallFlow` instead
# of the online `InstallWizard`).
#
# Output:
#   target/release/bundle/macos/ClawLite.app
#   target/release/bundle/dmg/ClawLite_<version>_<arch>.dmg
#
# Usage:
#   bash scripts/build-lite-macos.sh
#   bash scripts/build-lite-macos.sh --copy-to <dir>
set -euo pipefail

COPY_TO=""
while [ $# -gt 0 ]; do
    case "$1" in
        --copy-to)
            [ $# -ge 2 ] || { echo "ERROR: --copy-to needs a directory"; exit 1; }
            COPY_TO="$2"; shift 2 ;;
        --help|-h)
            echo "Usage: bash scripts/build-lite-macos.sh [--copy-to <dir>]"
            exit 0 ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

echo "=========================================="
echo "  ClawLite macOS Build"
echo "=========================================="

# Tauri needs rustc 1.88+. Prefer ~/.cargo/bin/rustc when system rustc is older.
if [ -f "$HOME/.cargo/bin/rustc" ]; then
    CARGO_VER=$("$HOME/.cargo/bin/rustc" --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
    CARGO_MINOR=$(echo "$CARGO_VER" | cut -d. -f2)
    if [ "$CARGO_MINOR" -ge 88 ]; then
        export PATH="$HOME/.cargo/bin:$PATH"
    fi
fi

# Install npm deps (main — lite shares the same frontend build).
npm install --no-audit --no-fund

# Tauri CLI's --config treats the config file's parent directory as the
# Tauri project root and expects a Cargo.toml there. `lite/` has no
# Cargo.toml (we share tauri/ for the Rust side), so invoke from inside
# tauri/ with a relative path to ../lite/clawlite.tauri.conf.json.
( cd tauri && cargo tauri build --config ../lite/clawlite.tauri.conf.json )

DMG=$(find target/release/bundle/dmg -name "ClawLite_*.dmg" 2>/dev/null | head -1)
APP="target/release/bundle/macos/ClawLite.app"

echo ""
echo "--- Build artifacts ---"
[ -d "$APP" ] && echo "  App: $APP ($(du -sh "$APP" | awk '{print $1}'))"
[ -n "$DMG" ] && echo "  DMG: $DMG ($(du -sh "$DMG" | awk '{print $1}'))"

if [ -n "$COPY_TO" ]; then
    mkdir -p "$COPY_TO"
    if [ -d "$APP" ]; then
        rm -rf "$COPY_TO/ClawLite.app" 2>/dev/null
        ditto "$APP" "$COPY_TO/ClawLite.app"
    fi
    [ -n "$DMG" ] && cp "$DMG" "$COPY_TO/"
    echo ""
    echo "  Copied to: $COPY_TO"
fi

echo ""
echo "Build complete."
