#!/bin/ash
# ClawEnv — Alpine Linux sandbox initialization script
# Used by all sandbox backends after Alpine base is ready
set -e

echo "=== ClawEnv Alpine Init ==="

# Fix cgroup v2 unified mode (for Lima/WSL2)
if [ -f /etc/conf.d/cgroups ]; then
    sed -i 's/rc_cgroup_mode=.*/rc_cgroup_mode=unified/' /etc/conf.d/cgroups
    rc-update add cgroups boot 2>/dev/null || true
fi

# Install runtime dependencies
apk update
apk add --no-cache nodejs npm git curl bash ca-certificates

# Install OpenClaw (version passed as $1, defaults to latest)
OPENCLAW_VERSION="${1:-latest}"
npm install -g "openclaw@${OPENCLAW_VERSION}"

# Verify installation
openclaw --version

# Optional: install browser components (if $2 = "browser")
if [ "$2" = "browser" ]; then
    echo "=== Installing browser components ==="
    apk add --no-cache chromium xvfb-run x11vnc novnc websockify ttf-freefont
fi

echo "=== ClawEnv Alpine Init Complete ==="
