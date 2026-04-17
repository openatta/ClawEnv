#!/bin/bash
# ClawEnv — macOS Build Script
#
# Builds the full Tauri app (GUI + CLI sidecar) for macOS.
# Outputs: tauri/target/release/bundle/dmg/*.dmg
#
# Prerequisites:
#   - Xcode Command Line Tools: xcode-select --install
#   - Rust (rustup): https://rustup.rs
#   - Node.js 20+: https://nodejs.org
#   - npm (comes with Node.js)
#
# Usage:
#   bash scripts/build-macos.sh              # Full release build
#   bash scripts/build-macos.sh --dev        # Dev build (unoptimized, faster)
#   bash scripts/build-macos.sh --cli-only   # Build CLI binary only (no GUI)
#
set -euo pipefail

MODE="release"
CLI_ONLY=false
for arg in "$@"; do
    case "$arg" in
        --dev) MODE="dev" ;;
        --cli-only) CLI_ONLY=true ;;
        --help|-h)
            echo "Usage: bash scripts/build-macos.sh [--dev] [--cli-only]"
            echo "  --dev       Dev build (debug profile, faster compilation)"
            echo "  --cli-only  Build CLI binary only, skip Tauri GUI"
            exit 0 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

echo "============================================"
echo "  ClawEnv macOS Build ($MODE)"
echo "============================================"
echo ""

# --- 1. Check prerequisites ---
echo "--- Checking prerequisites ---"

check_cmd() {
    if ! command -v "$1" &>/dev/null; then
        echo "ERROR: $1 not found. $2"
        exit 1
    fi
}

check_cmd rustc "Install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
check_cmd cargo "Install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
check_cmd node  "Install Node.js 20+: https://nodejs.org"
check_cmd npm   "Comes with Node.js"

# Tauri needs rustc 1.88+ (darling/time crates). Check ~/.cargo/bin/rustc first.
RUSTC_BIN="rustc"
if [ -f "$HOME/.cargo/bin/rustc" ]; then
    RUSTC_BIN="$HOME/.cargo/bin/rustc"
    export PATH="$HOME/.cargo/bin:$PATH"
fi
RUSTC_VER=$($RUSTC_BIN --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
RUSTC_MINOR=$(echo "$RUSTC_VER" | cut -d. -f2)
if [ "$RUSTC_MINOR" -lt 88 ] && [ "$CLI_ONLY" = false ]; then
    echo "WARNING: rustc $RUSTC_VER detected. Tauri build needs 1.88+."
    echo "  Checking ~/.cargo/bin/rustc..."
    if [ -f "$HOME/.cargo/bin/rustc" ]; then
        CARGO_VER=$("$HOME/.cargo/bin/rustc" --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')
        echo "  Found rustc $CARGO_VER in ~/.cargo/bin/"
        export PATH="$HOME/.cargo/bin:$PATH"
    else
        echo "ERROR: Need rustc 1.88+ for Tauri. Run: rustup update stable"
        exit 1
    fi
fi

echo "  rustc: $(rustc --version)"
echo "  cargo: $(cargo --version)"
echo "  node:  $(node --version)"
echo "  npm:   $(npm --version)"
echo ""

# --- 2. Install frontend dependencies ---
if [ "$CLI_ONLY" = false ]; then
    echo "--- Installing frontend dependencies ---"
    npm install --no-audit --no-fund
    echo ""
fi

# --- 3. Build ---
if [ "$CLI_ONLY" = true ]; then
    echo "--- Building CLI only ---"
    if [ "$MODE" = "release" ]; then
        cargo build -p clawcli --release
        BIN="target/release/clawcli"
    else
        cargo build -p clawcli
        BIN="target/debug/clawcli"
    fi
    echo ""
    echo "  CLI binary: $BIN"
    echo "  Version: $(./$BIN --version)"
    ls -lh "$BIN"
else
    echo "--- Building Tauri app ($MODE) ---"
    if [ "$MODE" = "release" ]; then
        npx tauri build 2>&1
    else
        # Dev build: just compile, don't package
        cargo build -p clawcli
        node scripts/copy-cli-sidecar.cjs debug
        npm run build
        cargo build -p clawgui
        echo ""
        echo "  Dev build complete. Run with: npx tauri dev"
    fi
fi

echo ""

# --- 4. Output ---
if [ "$MODE" = "release" ] && [ "$CLI_ONLY" = false ]; then
    echo "--- Build artifacts ---"
    DMG=$(find tauri/target/release/bundle/dmg -name "*.dmg" 2>/dev/null | head -1)
    if [ -n "$DMG" ]; then
        echo "  DMG: $DMG"
        ls -lh "$DMG"
    else
        echo "  WARNING: DMG not found. Check tauri/target/release/bundle/"
        ls tauri/target/release/bundle/ 2>/dev/null || true
    fi
fi

echo ""
echo "Build complete."
