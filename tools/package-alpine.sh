#!/bin/bash
# ClawEnv — Package an Alpine sandbox instance as a distributable image
#
# Usage:
#   bash tools/package-alpine.sh [OPTIONS] [instance_name] [output_dir]
#
# Options:
#   --chromium    Include Chromium + noVNC (adds ~630MB, default: skip)
#
# Examples:
#   bash tools/package-alpine.sh                          # default instance, no chromium
#   bash tools/package-alpine.sh --chromium default ./dist
#   bash tools/package-alpine.sh my-instance              # custom instance
#
# Platform behavior:
#   macOS (Lima):   exports Lima VM as tar.gz (qcow2/vz disk)
#   Linux (Podman): exports container as OCI image tar.gz
#   Windows (WSL2): exports distro as rootfs tar.gz

set -e

# Parse options
INSTALL_CHROMIUM=false
POSITIONAL=()
for arg in "$@"; do
    case "$arg" in
        --chromium) INSTALL_CHROMIUM=true ;;
        *)          POSITIONAL+=("$arg") ;;
    esac
done

INSTANCE="${POSITIONAL[0]:-default}"
OUTPUT_DIR="${POSITIONAL[1]:-./packages}"
VM_NAME="clawenv-${INSTANCE}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)

mkdir -p "$OUTPUT_DIR"

echo "=========================================="
echo "  ClawEnv Alpine Package Builder"
echo "=========================================="
echo "Instance:   $INSTANCE"
echo "VM Name:    $VM_NAME"
echo "Chromium:   $INSTALL_CHROMIUM"
echo "Output:     $OUTPUT_DIR"
echo ""

detect_platform() {
    case "$(uname -s)" in
        Darwin*)  echo "macos" ;;
        Linux*)   echo "linux" ;;
        CYGWIN*|MINGW*|MSYS*) echo "windows" ;;
        *)        echo "unknown" ;;
    esac
}

PLATFORM=$(detect_platform)
echo "Platform:   $PLATFORM"
echo ""

# Install Chromium if requested
if [ "$INSTALL_CHROMIUM" = true ]; then
    echo "=== Installing Chromium + noVNC ==="
    case "$PLATFORM" in
        macos)   limactl shell "$VM_NAME" -- sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont ;;
        linux)   podman exec "$VM_NAME" sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont ;;
        windows) wsl -d "$VM_NAME" -- sudo apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont ;;
    esac
    echo "Chromium installed."
    echo ""
fi

# Detect claw version
get_claw_version() {
    local cmd="for bin in openclaw zeroclaw autoclaw; do which \$bin >/dev/null 2>&1 && echo \"\$bin \$(\$bin --version 2>/dev/null || echo unknown)\" && exit 0; done; echo 'unknown unknown'"
    case "$PLATFORM" in
        macos)   limactl shell "$VM_NAME" -- sh -c "$cmd" 2>/dev/null | head -1 ;;
        linux)   podman exec "$VM_NAME" sh -c "$cmd" 2>/dev/null | head -1 ;;
        windows) wsl -d "$VM_NAME" -- sh -c "$cmd" 2>/dev/null | head -1 ;;
    esac
}

CLAW_INFO=$(get_claw_version)
CLAW_BIN=$(echo "$CLAW_INFO" | awk '{print $1}')
CLAW_VERSION=$(echo "$CLAW_INFO" | awk '{print $2}')
echo "Claw:       $CLAW_BIN $CLAW_VERSION"
echo ""

case "$PLATFORM" in
    macos)
        echo "=== Exporting Lima VM ==="
        limactl stop "$VM_NAME" 2>/dev/null || true
        sleep 2

        LIMA_DIR="$HOME/.lima/$VM_NAME"
        [ -d "$LIMA_DIR" ] || { echo "ERROR: Lima instance not found at $LIMA_DIR"; exit 1; }

        OUTFILE="$OUTPUT_DIR/clawenv-${INSTANCE}-${TIMESTAMP}-macos-$(uname -m).tar.gz"
        TAR_CMD="tar"; TAR_SPARSE=""
        command -v gtar &>/dev/null && { TAR_CMD="gtar"; TAR_SPARSE="--sparse"; }

        $TAR_CMD $TAR_SPARSE -czf "$OUTFILE" \
            -C "$HOME/.lima" \
            --exclude="$VM_NAME/*.sock" --exclude="$VM_NAME/*.pid" \
            --exclude="$VM_NAME/*.log" --exclude="$VM_NAME/cidata.iso" \
            --exclude="$VM_NAME/ssh.config" --exclude="$VM_NAME/cloud-config.yaml" \
            "$VM_NAME/"

        limactl start "$VM_NAME" 2>/dev/null &
        ;;

    linux)
        echo "=== Exporting Podman container ==="
        OUTFILE="$OUTPUT_DIR/clawenv-${INSTANCE}-${TIMESTAMP}-linux-$(uname -m).tar.gz"
        podman commit "$VM_NAME" "clawenv-export:${INSTANCE}"
        podman save -o "${OUTFILE%.gz}" "clawenv-export:${INSTANCE}"
        gzip "${OUTFILE%.gz}"
        ;;

    windows)
        echo "=== Exporting WSL2 distro ==="
        OUTFILE="$OUTPUT_DIR/clawenv-${INSTANCE}-${TIMESTAMP}-windows-$(uname -m).tar.gz"
        wsl --export "$VM_NAME" "$OUTFILE"
        ;;

    *) echo "ERROR: Unsupported platform: $PLATFORM"; exit 1 ;;
esac

# Manifest
FILESIZE=$(stat -f%z "$OUTFILE" 2>/dev/null || stat -c%s "$OUTFILE" 2>/dev/null || echo 0)
SHA256=$(shasum -a 256 "$OUTFILE" 2>/dev/null | awk '{print $1}' || sha256sum "$OUTFILE" 2>/dev/null | awk '{print $1}' || echo "unknown")

MANIFEST="$OUTPUT_DIR/manifest-${INSTANCE}-${TIMESTAMP}.toml"
cat > "$MANIFEST" << EOF
[package]
instance = "$INSTANCE"
platform = "$PLATFORM"
arch = "$(uname -m)"
created_at = "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
claw_binary = "$CLAW_BIN"
claw_version = "$CLAW_VERSION"
chromium = $INSTALL_CHROMIUM

[image]
file = "$(basename "$OUTFILE")"
size_bytes = $FILESIZE
sha256 = "$SHA256"

[clawenv]
version = "0.2.0"
EOF

echo ""
echo "=== Package complete ==="
ls -lh "$OUTFILE"
echo "Manifest: $MANIFEST"
echo ""
echo "Install on another machine:"
echo "  clawenv install --image $(basename "$OUTFILE")"
