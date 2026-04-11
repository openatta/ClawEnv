#!/bin/ash
# ClawEnv — Alpine Linux sandbox initialization script
# Used by all sandbox backends after Alpine base is ready
#
# Usage: init.sh [claw_package] [claw_version] [browser]
#   claw_package: npm package name (default: openclaw)
#   claw_version: version tag (default: latest)
#   browser:      "browser" to install Chromium + noVNC (optional)
set -e

CLAW_PACKAGE="${1:-openclaw}"
CLAW_VERSION="${2:-latest}"

echo "=== ClawEnv Alpine Init ==="
echo "  Claw: ${CLAW_PACKAGE}@${CLAW_VERSION}"

# Fix cgroup v2 unified mode (for Lima/WSL2)
if [ -f /etc/conf.d/cgroups ]; then
    sed -i 's/rc_cgroup_mode=.*/rc_cgroup_mode=unified/' /etc/conf.d/cgroups
    rc-update add cgroups boot 2>/dev/null || true
fi

# Install runtime dependencies
apk update
apk add --no-cache nodejs npm git curl bash ca-certificates

# Install the claw product
npm install -g "${CLAW_PACKAGE}@${CLAW_VERSION}"

# Verify installation (extract binary name from package — last segment after /)
CLAW_BIN=$(echo "$CLAW_PACKAGE" | sed 's|.*/||')
"$CLAW_BIN" --version 2>/dev/null || echo "Warning: ${CLAW_BIN} --version failed (non-standard CLI)"

# Optional: install browser components
if [ "$3" = "browser" ]; then
    echo "=== Installing browser components ==="
    apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont
fi

echo "=== ClawEnv Alpine Init Complete ==="
