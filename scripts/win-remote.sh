#!/bin/bash
# ClawEnv — Windows ARM64 remote build helper
# Usage: bash scripts/win-remote.sh <command>
#
# Commands:
#   test      — cargo test -p clawenv-core
#   check     — cargo check -p clawenv-core
#   build     — cargo tauri build
#   dev       — cargo tauri dev (needs GUI on Windows side)
#   pull      — git pull
#   shell     — open interactive SSH
#   run <cmd> — run arbitrary command
#
# Reads .env for WIN_HOST/WIN_USER

set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Load .env
if [ -f "$SCRIPT_DIR/../.env" ]; then
    export $(grep -v '^#' "$SCRIPT_DIR/../.env" | xargs)
fi

WIN_HOST="${WIN_HOST:-192.168.64.7}"
WIN_USER="${WIN_USER:-clawenv}"
# Project path on the Windows box — configured in .env as WIN_PROJECT.
# Default: %USERPROFILE%\ClawEnv (clawenv user's home).
WIN_PROJECT="${WIN_PROJECT:-C:\\Users\\$WIN_USER\\ClawEnv}"
PROJECT="$WIN_PROJECT"
CARGO="C:\\Users\\$WIN_USER\\.cargo\\bin\\cargo.exe"

# All tools must be in PATH for every SSH session (Node, Git, MSVC, LLVM, Cargo)
ENV_PREFIX="set PATH=%PATH%;C:\\Program Files\\nodejs;C:\\Program Files\\Git\\cmd;C:\\Program Files\\LLVM\\bin;C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Tools\\MSVC\\14.44.35207\\bin\\Hostx64\\x64;C:\\Users\\$WIN_USER\\.cargo\\bin&&"

win_run() {
    ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" "${ENV_PREFIX} cd $PROJECT && $*" 2>&1
}

CMD="${1:-help}"
shift 2>/dev/null || true

case "$CMD" in
    test)
        echo "=== Running tests on Windows ARM64 ==="
        win_run "$CARGO test -p clawenv-core $*"
        ;;
    check)
        echo "=== Cargo check on Windows ARM64 ==="
        win_run "$CARGO check -p clawenv-core $*"
        ;;
    build)
        echo "=== Cargo tauri build on Windows ARM64 ==="
        win_run "$CARGO tauri build $*"
        ;;
    dev)
        echo "=== Cargo tauri dev on Windows ARM64 ==="
        echo "(GUI will appear on the Windows desktop)"
        win_run "$CARGO tauri dev $*"
        ;;
    pull)
        echo "=== Git pull on Windows ==="
        win_run "\"C:\\Program Files\\Git\\cmd\\git.exe\" pull $*"
        ;;
    npm)
        echo "=== npm on Windows ==="
        win_run "\"C:\\Program Files\\nodejs\\npm.cmd\" $*"
        ;;
    shell)
        echo "=== Interactive SSH to Windows ==="
        ssh "$WIN_USER@$WIN_HOST"
        ;;
    run)
        win_run "$*"
        ;;
    sync)
        # Mirror the local source tree into $WIN_PROJECT on the Windows box via
        # a tar pipe over SSH. rsync isn't available natively on Windows and
        # Git-for-Windows's rsync.exe isn't shipped; tar over SSH is the next
        # best thing. We use Windows' built-in `tar.exe` (system32, BSD tar
        # with gzip support built in) so we don't depend on a separate gzip.
        # Git-for-Windows tar would try to `exec gzip` which isn't on PATH.
        # Excludes heavy / machine-specific dirs (target, node_modules, dist,
        # .git). First run creates the target dir; subsequent runs overwrite.
        echo "=== Sync local source → $PROJECT on Windows ==="
        LOCAL_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
        TAR_BIN='C:\\Windows\\System32\\tar.exe'
        # Create target dir if missing. Use cmd mkdir (accepts forward slashes).
        ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
            "if not exist \"$PROJECT\" mkdir \"$PROJECT\"" 2>&1 | grep -v "post-quantum" || true
        # Stream a gzip'd tar over SSH. BSD tar on Windows auto-detects gzip.
        # Set COPYFILE_DISABLE=1 so macOS's bsdtar doesn't serialize extended
        # attributes as separate `./._name` entries — Windows reads those as
        # regular files and chokes on the non-UTF-8 AppleDouble header
        # (e.g. "failed to read file 'capabilities\._default.json'").
        (cd "$LOCAL_ROOT" && COPYFILE_DISABLE=1 tar czf - \
            --exclude='./target' \
            --exclude='./node_modules' \
            --exclude='./dist' \
            --exclude='./.git' \
            --exclude='.DS_Store' \
            --exclude='._*' \
            .) | ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
            "cd \"$PROJECT\" && $TAR_BIN -xzf -" 2>&1 | grep -v "post-quantum" || true
        echo "Done. Source synced to $PROJECT"
        ;;
    help|*)
        echo "Usage: bash scripts/win-remote.sh <command>"
        echo ""
        echo "Commands:"
        echo "  test     Run cargo test -p clawenv-core"
        echo "  check    Run cargo check -p clawenv-core"
        echo "  build    Run cargo tauri build"
        echo "  dev      Run cargo tauri dev"
        echo "  pull     Git pull latest code"
        echo "  sync     Tar-stream local source (excluding target/node_modules/.git) to $WIN_PROJECT"
        echo "  npm      Run npm command (e.g., npm install)"
        echo "  shell    Open interactive SSH session"
        echo "  run      Run arbitrary command"
        ;;
esac
