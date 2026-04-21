#!/bin/bash
# Dev deploy (Windows): builds BOTH ClawEnv and ClawLite on the Windows
# UTM VM over SSH and pulls all four installers back to the Mac's
# ~/Desktop/ClawEnv/ directory alongside the macOS bundles.
#
# Mirrors dev-deploy-macos.sh's ergonomics so that a full-matrix
# deploy during debugging is:
#   bash scripts/dev-deploy-macos.sh
#   bash scripts/dev-deploy-windows.sh
#
# Expects .env with WIN_HOST / WIN_USER populated, and SSH key access
# to the VM. See scripts/win-remote.sh for connection details.
set -euo pipefail

DEST="${1:-$HOME/Desktop/ClawEnv}"
mkdir -p "$DEST"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

if [ -f "$PROJECT_DIR/.env" ]; then
    # shellcheck disable=SC2046
    export $(grep -v '^#' "$PROJECT_DIR/.env" | xargs)
fi
WIN_USER="${WIN_USER:-clawenv}"
WIN_HOST="${WIN_HOST:-}"
[ -n "$WIN_HOST" ] || { echo "WIN_HOST not set in .env"; exit 1; }

echo "=============================================="
echo "  Windows dev deploy — via $WIN_USER@$WIN_HOST"
echo "  Destination: $DEST"
echo "=============================================="

# 1) Sync local tree to the VM (tar over SSH — handled by win-remote.sh).
echo ""
echo "--- Sync local → Windows ---"
bash "$SCRIPT_DIR/win-remote.sh" sync

# 2) Kill any stale clawgui.exe so the new build can overwrite it.
# `taskkill` returns 128 when nothing matches — swallow with || true.
bash "$SCRIPT_DIR/win-remote.sh" run \
    "taskkill /F /IM clawgui.exe /T 2>nul || echo ok" >/dev/null 2>&1 || true

# 3) Build ClawEnv (main) on Windows.
echo ""
echo "--- Building ClawEnv on Windows ---"
bash "$SCRIPT_DIR/win-remote.sh" build

# 4) Build ClawLite on Windows (merged config; from tauri/ so Tauri CLI
# finds Cargo.toml — see docs/06-lite.md for the rename rationale).
echo ""
echo "--- Building ClawLite on Windows ---"
bash "$SCRIPT_DIR/win-remote.sh" run \
    "cd tauri && C:\\Users\\$WIN_USER\\.cargo\\bin\\cargo.exe tauri build --config ..\\lite\\clawlite.tauri.conf.json"

# 5) scp all installers back to the Mac desktop folder.
# Version string comes from core/Cargo.toml so the script doesn't go
# stale on every release bump (we shipped 0.3.2 once with 0.3.1
# hardcoded paths — scp silently copied nothing and left last-release
# files on disk). Paths use forward slashes — OpenSSH scp handles them
# fine even on Windows targets.
echo ""
echo "--- Copying artifacts back to $DEST ---"
VERSION=$(awk '/^\[package\]/{p=1} p && /^version/{gsub(/[" ]/,"",$3); print $3; exit}' "$PROJECT_DIR/core/Cargo.toml")
[ -n "$VERSION" ] || { echo "ERROR: couldn't parse version from core/Cargo.toml"; exit 1; }
REMOTE_ROOT="C:/Users/$WIN_USER/ClawEnv"
for src in \
    "target/release/bundle/msi/ClawEnv_${VERSION}_arm64_en-US.msi" \
    "target/release/bundle/msi/ClawLite_${VERSION}_arm64_en-US.msi" \
    "target/release/bundle/nsis/ClawEnv_${VERSION}_arm64-setup.exe" \
    "target/release/bundle/nsis/ClawLite_${VERSION}_arm64-setup.exe"; do
    scp -o ConnectTimeout=10 -q "$WIN_USER@$WIN_HOST:$REMOTE_ROOT/$src" "$DEST/"
    base=$(basename "$src")
    if [ -f "$DEST/$base" ]; then
        echo "  $base  $(du -sh "$DEST/$base" | awk '{print $1}')"
    else
        echo "  MISSING: $base"
    fi
done

echo ""
echo "Deploy complete. Windows artifacts landed in $DEST"
