#!/bin/bash
# ClawEnv — Package a native (no-sandbox) bundle for offline installation
#
# Usage:
#   bash tools/package-native.sh [OPTIONS] [openclaw_version] [output_dir]
#
# Options:
#   --chromium    Ignored for native mode (native uses system browser)
#
# Examples:
#   bash tools/package-native.sh                          # latest version
#   bash tools/package-native.sh 2026.4.12 ./dist
#   bash tools/package-native.sh --chromium latest        # --chromium is ignored
#
# Mirror env vars (for China / behind firewall):
#   NODEJS_DIST_MIRROR=https://npmmirror.com/mirrors/node
#   NPM_REGISTRY_MIRROR=https://registry.npmmirror.com
#
# Output: clawenv-native-{version}-{os}-{arch}.tar.gz

set -e

# Parse options
POSITIONAL=()
for arg in "$@"; do
    case "$arg" in
        --chromium) echo "Note: --chromium ignored for native mode (uses system browser)" ;;
        *)          POSITIONAL+=("$arg") ;;
    esac
done

OC_VERSION="${POSITIONAL[0]:-latest}"
OUTPUT_DIR="${POSITIONAL[1]:-./packages}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
NODE_VERSION="v22.16.0"

NODEJS_DIST="${NODEJS_DIST_MIRROR:-https://nodejs.org/dist}"
NPM_REGISTRY="${NPM_REGISTRY_MIRROR:-}"

mkdir -p "$OUTPUT_DIR"

detect_platform() {
    case "$(uname -s 2>/dev/null)" in
        Darwin*)  echo "macos" ;;
        Linux*)   echo "linux" ;;
        CYGWIN*|MINGW*|MSYS*) echo "windows" ;;
        *)
            [ -n "$USERPROFILE" ] || [ -n "$WINDIR" ] && echo "windows" || echo "unknown"
            ;;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)   echo "x64" ;;
        aarch64|arm64)  echo "arm64" ;;
        *)              echo "$(uname -m)" ;;
    esac
}

PLATFORM=$(detect_platform)
ARCH=$(detect_arch)

echo "=========================================="
echo "  ClawEnv Native Bundle Builder"
echo "=========================================="
echo "Platform:   $PLATFORM / $ARCH"
echo "Node.js:    $NODE_VERSION"
echo "OpenClaw:   $OC_VERSION"
echo "Output:     $OUTPUT_DIR"
echo ""

BUILD_DIR=$(mktemp -d)
trap 'rm -rf "$BUILD_DIR"' EXIT

# Step 1: Download Node.js
echo "=== Step 1: Download Node.js ==="
case "$PLATFORM" in
    macos)   NODE_FILENAME="node-${NODE_VERSION}-darwin-${ARCH}.tar.gz" ;;
    linux)   NODE_FILENAME="node-${NODE_VERSION}-linux-${ARCH}.tar.xz" ;;
    windows) NODE_FILENAME="node-${NODE_VERSION}-win-${ARCH}.zip" ;;
    *)       echo "ERROR: Unsupported platform"; exit 1 ;;
esac

curl -fSL -o "$BUILD_DIR/$NODE_FILENAME" "${NODEJS_DIST}/${NODE_VERSION}/${NODE_FILENAME}"

mkdir -p "$BUILD_DIR/node"
case "$PLATFORM" in
    macos|linux)
        if [[ "$NODE_FILENAME" == *.tar.xz ]]; then
            tar xJf "$BUILD_DIR/$NODE_FILENAME" --strip-components=1 -C "$BUILD_DIR/node"
        else
            tar xzf "$BUILD_DIR/$NODE_FILENAME" --strip-components=1 -C "$BUILD_DIR/node"
        fi
        ;;
    windows)
        WIN_BUILD_DIR=$(cygpath -w "$BUILD_DIR" 2>/dev/null || echo "$BUILD_DIR")
        powershell -Command "Expand-Archive -Path '$WIN_BUILD_DIR\\$NODE_FILENAME' -DestinationPath '$WIN_BUILD_DIR\\node-tmp'"
        mv "$BUILD_DIR/node-tmp"/node-*/* "$BUILD_DIR/node/" 2>/dev/null || true
        rm -rf "$BUILD_DIR/node-tmp"
        ;;
esac
echo "Node.js ready."

# Step 2: Install OpenClaw
echo "=== Step 2: Install OpenClaw@$OC_VERSION ==="
export PATH="$BUILD_DIR/node/bin:$PATH"
export npm_config_prefix="$BUILD_DIR"
[ -n "$NPM_REGISTRY" ] && npm config set registry "$NPM_REGISTRY"

npm install -g "openclaw@${OC_VERSION}" --loglevel info
ACTUAL_VERSION=$(openclaw --version 2>/dev/null || echo "$OC_VERSION")
echo "OpenClaw $ACTUAL_VERSION installed."

# Reorganize layout
mkdir -p "$BUILD_DIR/node_modules"
[ -d "$BUILD_DIR/lib/node_modules" ] && mv "$BUILD_DIR/lib/node_modules"/* "$BUILD_DIR/node_modules/" 2>/dev/null && rm -rf "$BUILD_DIR/lib"
[ -d "$BUILD_DIR/bin" ] && { mkdir -p "$BUILD_DIR/node_modules/.bin"; cp -a "$BUILD_DIR/bin"/* "$BUILD_DIR/node_modules/.bin/" 2>/dev/null; rm -rf "$BUILD_DIR/bin"; }

# Step 3: Package
echo "=== Step 3: Packaging ==="
rm -f "$BUILD_DIR/$NODE_FILENAME"
rm -rf "$BUILD_DIR/share" "$BUILD_DIR/include"

OUTFILE="$OUTPUT_DIR/clawenv-native-${ACTUAL_VERSION}-${PLATFORM}-${ARCH}.tar.gz"

cat > "$BUILD_DIR/manifest.toml" << EOF
[bundle]
type = "native"
platform = "$PLATFORM"
arch = "$ARCH"
created_at = "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
[node]
version = "$NODE_VERSION"
[openclaw]
version = "$ACTUAL_VERSION"
[clawenv]
min_version = "0.2.0"
EOF

tar czf "$OUTFILE" -C "$BUILD_DIR" node node_modules manifest.toml

SHA256=$(shasum -a 256 "$OUTFILE" 2>/dev/null | awk '{print $1}' || sha256sum "$OUTFILE" 2>/dev/null | awk '{print $1}' || echo "unknown")

echo ""
echo "=== Bundle complete ==="
ls -lh "$OUTFILE"
echo "SHA256: $SHA256"
echo ""
echo "Install: clawenv install --native-bundle $(basename "$OUTFILE")"
