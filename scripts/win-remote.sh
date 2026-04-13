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
PROJECT="C:\\Users\\$WIN_USER\\Desktop\\ClawEnv"
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
    help|*)
        echo "Usage: bash scripts/win-remote.sh <command>"
        echo ""
        echo "Commands:"
        echo "  test     Run cargo test -p clawenv-core"
        echo "  check    Run cargo check -p clawenv-core"
        echo "  build    Run cargo tauri build"
        echo "  dev      Run cargo tauri dev"
        echo "  pull     Git pull latest code"
        echo "  npm      Run npm command (e.g., npm install)"
        echo "  shell    Open interactive SSH session"
        echo "  run      Run arbitrary command"
        ;;
esac
