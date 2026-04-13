#!/bin/bash
# ClawEnv — Package an Alpine sandbox instance as a distributable image
#
# Usage:
#   bash tools/package-alpine.sh [instance_name] [output_dir]
#
# Examples:
#   bash tools/package-alpine.sh default ./dist-packages
#   bash tools/package-alpine.sh              # defaults: instance=default, output=./packages
#
# This script detects the current platform and exports the sandbox
# in the appropriate format:
#   macOS (Lima):   qcow2 disk image
#   Linux (Podman): OCI container image tar
#   Windows (WSL2): rootfs tar.gz

set -e

INSTANCE="${1:-default}"
OUTPUT_DIR="${2:-./packages}"
VM_NAME="clawenv-${INSTANCE}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)

mkdir -p "$OUTPUT_DIR"

echo "=========================================="
echo "  ClawEnv Package Builder"
echo "=========================================="
echo ""
echo "Instance:   $INSTANCE"
echo "VM Name:    $VM_NAME"
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

# Detect which claw binary is installed inside the sandbox
get_claw_version() {
    local cmd="for bin in openclaw zeroclaw autoclaw qclaw kimi-claw easyclaw duclaw arkclaw maxclaw chatclaw; do which \$bin >/dev/null 2>&1 && echo \"\$bin \$(\$bin --version 2>/dev/null || echo unknown)\" && exit 0; done; echo 'unknown unknown'"
    case "$PLATFORM" in
        macos)
            limactl shell "$VM_NAME" -- sh -c "$cmd" 2>/dev/null | head -1
            ;;
        linux)
            podman exec "$VM_NAME" sh -c "$cmd" 2>/dev/null | head -1
            ;;
        windows)
            wsl -d "ClawEnv-Alpine" -- sh -c "$cmd" 2>/dev/null | head -1
            ;;
    esac
}

CLAW_INFO=$(get_claw_version)
CLAW_BIN=$(echo "$CLAW_INFO" | awk '{print $1}')
CLAW_VERSION=$(echo "$CLAW_INFO" | awk '{print $2}')
echo "Claw:       $CLAW_BIN $CLAW_VERSION"
echo ""

case "$PLATFORM" in
    macos)
        echo "=== Exporting Lima VM as disk image ==="
        # Stop VM first for clean export
        echo "Stopping VM..."
        limactl stop "$VM_NAME" 2>/dev/null || true
        sleep 2

        # Copy the disk image
        LIMA_DIR="$HOME/.lima/$VM_NAME"
        if [ ! -d "$LIMA_DIR" ]; then
            echo "ERROR: Lima instance '$VM_NAME' not found at $LIMA_DIR"
            exit 1
        fi

        OUTFILE="$OUTPUT_DIR/clawenv-${INSTANCE}-${TIMESTAMP}-macos-$(uname -m).tar.gz"

        echo "Packaging Lima instance..."
        # Export essential files: disk + config + EFI (excludes logs, sockets, pids)
        # Supports both QEMU (diffdisk/basedisk) and VZ (disk, vz-efi) drivers.
        # GNU tar supports --sparse for efficient packing of VZ sparse disk files.
        TAR_CMD="tar"
        TAR_SPARSE=""
        if command -v gtar &>/dev/null; then
            TAR_CMD="gtar"
            TAR_SPARSE="--sparse"
        fi

        $TAR_CMD $TAR_SPARSE -czf "$OUTFILE" \
            -C "$HOME/.lima" \
            --exclude="$VM_NAME/*.sock" \
            --exclude="$VM_NAME/*.pid" \
            --exclude="$VM_NAME/*.log" \
            --exclude="$VM_NAME/cidata.iso" \
            --exclude="$VM_NAME/ssh.config" \
            --exclude="$VM_NAME/cloud-config.yaml" \
            "$VM_NAME/"

        if [ $? -ne 0 ]; then
            echo "ERROR: Failed to package Lima instance"
            echo "TIP: Install GNU tar for better sparse file handling: brew install gnu-tar"
            exit 1
        fi

        # Restart VM
        echo "Restarting VM..."
        limactl start "$VM_NAME" 2>/dev/null &

        echo ""
        echo "=== Package created ==="
        ls -lh "$OUTFILE"
        ;;

    linux)
        echo "=== Exporting Podman container as OCI image ==="
        OUTFILE="$OUTPUT_DIR/clawenv-${INSTANCE}-${TIMESTAMP}-linux-$(uname -m).tar"

        # Commit running container to image
        echo "Committing container state..."
        podman commit "$VM_NAME" "clawenv-export:${INSTANCE}" 2>/dev/null

        # Save as tar
        echo "Saving image..."
        podman save -o "$OUTFILE" "clawenv-export:${INSTANCE}"

        # Compress
        echo "Compressing..."
        gzip "$OUTFILE"
        OUTFILE="${OUTFILE}.gz"

        echo ""
        echo "=== Package created ==="
        ls -lh "$OUTFILE"
        ;;

    windows)
        echo "=== Exporting WSL2 distro as tar.gz ==="
        OUTFILE="$OUTPUT_DIR/clawenv-${INSTANCE}-${TIMESTAMP}-windows-$(uname -m).tar.gz"

        echo "Exporting WSL distro..."
        wsl --export "ClawEnv-Alpine" "$OUTFILE"

        echo ""
        echo "=== Package created ==="
        ls -lh "$OUTFILE"
        ;;

    *)
        echo "ERROR: Unsupported platform: $PLATFORM"
        exit 1
        ;;
esac

# Generate manifest
MANIFEST="$OUTPUT_DIR/manifest-${INSTANCE}-${TIMESTAMP}.toml"
FILESIZE=$(stat -f%z "$OUTFILE" 2>/dev/null || stat -c%s "$OUTFILE" 2>/dev/null || echo 0)
SHA256=$(shasum -a 256 "$OUTFILE" 2>/dev/null | awk '{print $1}' || sha256sum "$OUTFILE" 2>/dev/null | awk '{print $1}' || echo "unknown")

cat > "$MANIFEST" << EOF
# ClawEnv Package Manifest
[package]
instance = "$INSTANCE"
platform = "$PLATFORM"
arch = "$(uname -m)"
created_at = "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
claw_binary = "$CLAW_BIN"
claw_version = "$CLAW_VERSION"

[image]
file = "$(basename "$OUTFILE")"
size_bytes = $FILESIZE
sha256 = "$SHA256"

[clawenv]
version = "0.2.0"
EOF

echo ""
echo "Manifest: $MANIFEST"
cat "$MANIFEST"
echo ""
echo "=========================================="
echo "  Package complete!"
echo "=========================================="
echo ""
echo "To install on another machine:"
echo "  clawenv install --image $(basename "$OUTFILE")"
