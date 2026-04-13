#!/bin/bash
# ClawEnv — Package a native (no-sandbox) bundle for offline installation
#
# Usage:
#   bash tools/package-native.sh [openclaw_version] [output_dir]
#
# Examples:
#   bash tools/package-native.sh latest ./dist-packages
#   bash tools/package-native.sh 0.5.2
#   bash tools/package-native.sh              # defaults: version=latest, output=./packages
#
# For China / behind a firewall, set mirror env vars:
#   NODEJS_DIST_MIRROR=https://npmmirror.com/mirrors/node \
#   NPM_REGISTRY_MIRROR=https://registry.npmmirror.com \
#   bash tools/package-native.sh
#
# Creates a self-contained bundle containing:
#   node/          — Node.js runtime
#   node_modules/  — OpenClaw and all dependencies (pre-installed)
#   manifest.toml  — Bundle metadata
#
# Output: clawenv-native-{version}-{os}-{arch}.tar.gz
#
# Run this script on each target platform to produce platform-specific bundles.

set -e

OC_VERSION="${1:-latest}"
OUTPUT_DIR="${2:-./packages}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
NODE_VERSION="v22.16.0"

# Mirror overrides (set env vars before running for domestic mirrors)
#   NODEJS_DIST_MIRROR=https://npmmirror.com/mirrors/node
#   NPM_REGISTRY_MIRROR=https://registry.npmmirror.com
NODEJS_DIST="${NODEJS_DIST_MIRROR:-https://nodejs.org/dist}"
NPM_REGISTRY="${NPM_REGISTRY_MIRROR:-}"

mkdir -p "$OUTPUT_DIR"

# ---- Platform detection ----
detect_platform() {
    case "$(uname -s 2>/dev/null)" in
        Darwin*)  echo "macos" ;;
        Linux*)   echo "linux" ;;
        CYGWIN*|MINGW*|MSYS*) echo "windows" ;;
        *)
            # Fallback: check for Windows environment variables
            if [ -n "$USERPROFILE" ] || [ -n "$WINDIR" ]; then
                echo "windows"
            else
                echo "unknown"
            fi
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
echo ""
echo "Platform:       $PLATFORM"
echo "Arch:           $ARCH"
echo "Node.js:        $NODE_VERSION"
echo "OpenClaw:       $OC_VERSION"
echo "Output:         $OUTPUT_DIR"
echo ""

# ---- Build directory ----
BUILD_DIR=$(mktemp -d)
trap 'rm -rf "$BUILD_DIR"' EXIT

echo "Build directory: $BUILD_DIR"
echo ""

# ---- Step 1: Download Node.js ----
echo "=== Step 1: Downloading Node.js $NODE_VERSION ==="

case "$PLATFORM" in
    macos)
        NODE_FILENAME="node-${NODE_VERSION}-darwin-${ARCH}.tar.gz"
        NODE_URL="${NODEJS_DIST}/${NODE_VERSION}/${NODE_FILENAME}"
        ;;
    linux)
        NODE_FILENAME="node-${NODE_VERSION}-linux-${ARCH}.tar.xz"
        NODE_URL="${NODEJS_DIST}/${NODE_VERSION}/${NODE_FILENAME}"
        ;;
    windows)
        NODE_FILENAME="node-${NODE_VERSION}-win-${ARCH}.zip"
        NODE_URL="${NODEJS_DIST}/${NODE_VERSION}/${NODE_FILENAME}"
        ;;
    *)
        echo "ERROR: Unsupported platform: $PLATFORM"
        exit 1
        ;;
esac

echo "Downloading $NODE_URL ..."
curl -fSL -o "$BUILD_DIR/$NODE_FILENAME" "$NODE_URL"

echo "Extracting Node.js..."
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
        # Convert Git Bash paths to Windows paths for PowerShell
        WIN_BUILD_DIR=$(cygpath -w "$BUILD_DIR" 2>/dev/null || echo "$BUILD_DIR")
        powershell -Command "Expand-Archive -Path '$WIN_BUILD_DIR\\$NODE_FILENAME' -DestinationPath '$WIN_BUILD_DIR\\node-tmp'"
        # Move contents up one level (strip the version directory)
        mv "$BUILD_DIR/node-tmp"/node-*/* "$BUILD_DIR/node/" 2>/dev/null || true
        rm -rf "$BUILD_DIR/node-tmp"
        ;;
esac

# Verify
"$BUILD_DIR/node/bin/node" --version 2>/dev/null || "$BUILD_DIR/node/node.exe" --version 2>/dev/null
echo "Node.js ready."
echo ""

# ---- Step 2: Install OpenClaw globally into the bundle ----
echo "=== Step 2: Installing OpenClaw@$OC_VERSION ==="

# Set npm prefix to install into our build dir
export PATH="$BUILD_DIR/node/bin:$PATH"
export npm_config_prefix="$BUILD_DIR"

# Use custom npm registry if set
if [ -n "$NPM_REGISTRY" ]; then
    echo "Using npm registry: $NPM_REGISTRY"
    npm config set registry "$NPM_REGISTRY"
fi

npm install -g "openclaw@${OC_VERSION}" --loglevel info

# Resolve actual version
ACTUAL_VERSION=$(openclaw --version 2>/dev/null || echo "$OC_VERSION")
echo ""
echo "OpenClaw $ACTUAL_VERSION installed."
echo ""

# The global install puts binaries in $BUILD_DIR/bin/ and modules in $BUILD_DIR/lib/node_modules/
# Reorganize: move global modules to $BUILD_DIR/node_modules/ for cleaner layout
mkdir -p "$BUILD_DIR/node_modules"
if [ -d "$BUILD_DIR/lib/node_modules" ]; then
    mv "$BUILD_DIR/lib/node_modules"/* "$BUILD_DIR/node_modules/" 2>/dev/null || true
    rm -rf "$BUILD_DIR/lib"
fi
# Copy bin stubs
if [ -d "$BUILD_DIR/bin" ]; then
    mkdir -p "$BUILD_DIR/node_modules/.bin"
    cp -a "$BUILD_DIR/bin"/* "$BUILD_DIR/node_modules/.bin/" 2>/dev/null || true
    rm -rf "$BUILD_DIR/bin"
fi

# ---- Step 3: Write manifest ----
echo "=== Step 3: Writing manifest ==="

cat > "$BUILD_DIR/manifest.toml" << EOF
# ClawEnv Native Bundle Manifest
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
min_version = "0.9.0"
EOF

echo "Manifest written."
echo ""

# ---- Step 4: Package ----
echo "=== Step 4: Packaging bundle ==="

# Clean up download artifacts before packaging
rm -f "$BUILD_DIR/$NODE_FILENAME"
rm -rf "$BUILD_DIR/share" "$BUILD_DIR/include"  # npm docs, node headers

OUTFILE="$OUTPUT_DIR/clawenv-native-${ACTUAL_VERSION}-${PLATFORM}-${ARCH}.tar.gz"

tar czf "$OUTFILE" -C "$BUILD_DIR" node node_modules manifest.toml

echo ""
echo "=== Bundle created ==="
ls -lh "$OUTFILE"

# Generate SHA256
SHA256=$(shasum -a 256 "$OUTFILE" 2>/dev/null | awk '{print $1}' || sha256sum "$OUTFILE" 2>/dev/null | awk '{print $1}' || echo "unknown")
FILESIZE=$(stat -f%z "$OUTFILE" 2>/dev/null || stat -c%s "$OUTFILE" 2>/dev/null || echo 0)

echo ""
echo "SHA256:    $SHA256"
echo "Size:      $FILESIZE bytes"

# Write manifest alongside the bundle
MANIFEST="$OUTPUT_DIR/manifest-native-${ACTUAL_VERSION}-${PLATFORM}-${ARCH}.toml"
cat > "$MANIFEST" << EOF
# ClawEnv Native Bundle Manifest
[package]
type = "native"
platform = "$PLATFORM"
arch = "$ARCH"
created_at = "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
node_version = "$NODE_VERSION"
openclaw_version = "$ACTUAL_VERSION"

[image]
file = "$(basename "$OUTFILE")"
size_bytes = $FILESIZE
sha256 = "$SHA256"

[clawenv]
min_version = "0.9.0"
EOF

echo ""
echo "Manifest: $MANIFEST"
echo ""
echo "=========================================="
echo "  Native bundle complete!"
echo "=========================================="
echo ""
echo "To install on another machine:"
echo "  clawenv install --native-bundle $(basename "$OUTFILE")"
