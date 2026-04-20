#!/bin/bash
# Prewarm: seed the isolated test HOME with cached prerequisite binaries
# from the user's REAL ~/.clawenv. Each scenario gets an independent
# copy (cp -R, no symlinks) so concurrent runs and post-test teardowns
# don't cross-contaminate.
#
# Why this matters: scenarios run with HOME=/tmp/clawenv-e2e-smoke-XX
# starting empty. Without prewarm, every scenario's `ensure_prerequisites`
# tries to download lima (~50MB) / dugite (~57MB) / mingit / node from
# upstream. In a GFW network the github tarball can trickle at <1KB/s
# and stall 30+ minutes. The download throughput floor (download.rs)
# now triggers fallback after 30s, but the cleanest fix is to skip the
# download entirely when we already have the binary cached locally.
#
# Source: $E2E_REAL_HOME/.clawenv/{bin,share,git,node}
# Destination: $HOME/.clawenv/  (HOME is the isolated path set by isolate.sh)

# What to prewarm depends on scenario kind. Sandbox scenarios need
# limactl (and its share/lima data dir); native scenarios need git +
# node. We prewarm BOTH unconditionally — copying spare bytes is cheap
# vs. risking a download stall.
e2e_prewarm_seed_home() {
    local src="${E2E_REAL_HOME:-$HOME}/.clawenv"
    local dst="${HOME}/.clawenv"
    if [ ! -d "$src" ]; then
        echo "[prewarm] no real ~/.clawenv to seed from — scenario will download fresh" >&2
        return 0
    fi
    mkdir -p "$dst"

    # Lima binaries (sandbox) — bin/limactl, share/lima/, share/qemu/
    for sub in bin share/lima share/qemu; do
        if [ -d "$src/$sub" ]; then
            mkdir -p "$dst/$(dirname "$sub")"
            cp -R "$src/$sub" "$dst/$sub" 2>/dev/null || true
        fi
    done

    # Git binary tree (entirely safe — no leakage, just the git distro).
    if [ -d "$src/git" ]; then
        cp -R "$src/git" "$dst/git" 2>/dev/null || true
    fi

    # Node binary + npm only. We exclude every other package under
    # node/lib/node_modules/ because copying e.g. an existing `openclaw`
    # install would cause `desc.version_check_cmd()` to report "already
    # installed" and the scenario would skip the actual `npm install`
    # we're trying to test. Keep `npm` and `corepack` since they are
    # node's own package manager and toolchain, not 3rd-party.
    if [ -d "$src/node" ]; then
        # Top-level files + bin/include/share are safe verbatim.
        for kid in bin include share; do
            if [ -e "$src/node/$kid" ]; then
                mkdir -p "$dst/node"
                cp -R "$src/node/$kid" "$dst/node/$kid" 2>/dev/null || true
            fi
        done
        # Top-level files (LICENSE, CHANGELOG, etc).
        find "$src/node" -maxdepth 1 -type f -exec cp {} "$dst/node/" \; 2>/dev/null || true
        # node_modules: only the npm built-ins, never 3rd party.
        if [ -d "$src/node/lib/node_modules" ]; then
            mkdir -p "$dst/node/lib/node_modules"
            for pkg in npm corepack; do
                if [ -d "$src/node/lib/node_modules/$pkg" ]; then
                    cp -R "$src/node/lib/node_modules/$pkg" \
                          "$dst/node/lib/node_modules/$pkg" 2>/dev/null || true
                fi
            done
        fi
    fi

    # Cache dir (alpine rootfs, lima images) is safe — these are
    # download-byte-identical caches of upstream artifacts, not user
    # state. Skipping them is what the user asked: `不要 cache 不然不准`.
    # Keep DISABLED so npm/apk truly download fresh in each scenario.
    # if [ -d "$src/cache" ]; then cp -R "$src/cache" "$dst/cache"; fi

    local seeded=$(du -sh "$dst" 2>/dev/null | awk '{print $1}')
    echo "[prewarm] seeded $dst from $src (size: $seeded)" >&2
}
