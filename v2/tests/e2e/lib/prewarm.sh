#!/bin/bash
# Prewarm: seed the isolated test HOME with cached prerequisite binaries
# from the user's REAL ~/.clawenv. Each scenario gets an independent
# copy (cp -R, no symlinks) so concurrent runs and post-test teardowns
# don't cross-contaminate.
#
# Why this matters: scenarios run with HOME=/tmp/clawenv-e2e-... starting
# empty. Without prewarm, every scenario's `ensure_prerequisites` tries
# to download lima (~50MB) / dugite / mingit / node from upstream. In a
# GFW network the github tarball can trickle at <1KB/s and stall 30+
# minutes. The cleanest fix is to skip the download entirely when we
# already have the binary cached locally.
#
# Lifted from v1 tests/e2e/lib/prewarm.sh — same layout assumptions
# (~/.clawenv/{bin,share,git,node}) hold in v2.

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

    # Git binary tree.
    if [ -d "$src/git" ]; then
        cp -R "$src/git" "$dst/git" 2>/dev/null || true
    fi

    # Node binary + npm + corepack only. We exclude every other package
    # under node/lib/node_modules/ because copying e.g. an existing
    # `openclaw` install would cause version_check to report "already
    # installed" and the scenario would skip the actual `npm install`
    # we're trying to test.
    if [ -d "$src/node" ]; then
        for kid in bin include share; do
            if [ -e "$src/node/$kid" ]; then
                mkdir -p "$dst/node"
                cp -R "$src/node/$kid" "$dst/node/$kid" 2>/dev/null || true
            fi
        done
        find "$src/node" -maxdepth 1 -type f -exec cp {} "$dst/node/" \; 2>/dev/null || true
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

    local seeded=$(du -sh "$dst" 2>/dev/null | awk '{print $1}')
    echo "[prewarm] seeded $dst from $src (size: $seeded)" >&2
}
