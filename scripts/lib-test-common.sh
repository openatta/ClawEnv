#!/bin/bash
# ClawEnv — Shared test utilities
# Source this file from test scripts: source "$(dirname "$0")/lib-test-common.sh"

# ---- Platform ----
detect_platform() {
    case "$(uname -s)" in
        Darwin*)  echo "macos" ;;
        Linux*)   echo "linux" ;;
        CYGWIN*|MINGW*|MSYS*) echo "windows" ;;
        *)        echo "unsupported" ;;
    esac
}

# ---- Cross-platform timeout replacement ----
# Works on macOS (no coreutils), Linux, and Windows/MSYS.
# Usage: run_with_timeout <seconds> <command...>
# Returns: command exit code, or 124 if timed out.
run_with_timeout() {
    local timeout_sec="$1"; shift

    # Try native `timeout` first (Linux, brew coreutils on macOS)
    if command -v timeout >/dev/null 2>&1; then
        timeout "$timeout_sec" "$@"
        return $?
    fi
    # Try gtimeout (macOS with `brew install coreutils`)
    if command -v gtimeout >/dev/null 2>&1; then
        gtimeout "$timeout_sec" "$@"
        return $?
    fi

    # Fallback: pure-bash background + kill
    "$@" &
    local cmd_pid=$!

    # Watchdog in background
    (
        sleep "$timeout_sec"
        kill -TERM "$cmd_pid" 2>/dev/null
        sleep 2
        kill -KILL "$cmd_pid" 2>/dev/null
    ) &
    local watchdog_pid=$!

    # Wait for the command
    wait "$cmd_pid" 2>/dev/null
    local rc=$?

    # Kill the watchdog if command finished first
    kill "$watchdog_pid" 2>/dev/null
    wait "$watchdog_pid" 2>/dev/null

    # Detect if killed by signal (128+TERM=143, 128+KILL=137)
    if [ "$rc" -eq 143 ] || [ "$rc" -eq 137 ]; then
        return 124  # Mimic GNU timeout exit code
    fi
    return "$rc"
}

# ---- Parse a field from claw-registry.toml ----
# Usage: parse_registry_field <registry_file> <claw_id> <field_name>
# Handles multi-word quoted values like: gateway_cmd = "gateway --port {port} --allow-unconfigured"
parse_registry_field() {
    local registry="$1" id="$2" field="$3"
    awk -v id="$id" -v f="$field" '
        /^\[\[claw\]\]/ { found=0 }
        /^id = / { gsub(/"/, "", $3); if ($3 == id) found=1 }
        found && $0 ~ "^"f" = " {
            # Try quoted value first: field = "value here"
            if (match($0, /"[^"]*"/)) {
                print substr($0, RSTART+1, RLENGTH-2)
            } else {
                # Unquoted value (numbers, booleans): field = 3000
                sub(/^[^=]+= */, "")
                gsub(/^[ \t]+|[ \t]+$/, "")
                print
            }
            exit
        }
    ' "$registry"
}

# ---- Sandbox exec: run a command inside a VM/container ----
# Usage: sandbox_exec <platform> <vm_name> <command>
sandbox_exec() {
    local platform="$1" vm_name="$2"; shift 2
    case "$platform" in
        macos)   limactl shell "$vm_name" -- sh -c "$*" 2>/dev/null ;;
        linux)   podman exec "$vm_name" sh -c "$*" 2>/dev/null ;;
        windows) wsl -d "$vm_name" -- sh -c "$*" 2>/dev/null ;;
        *)       echo "unsupported platform" >&2; return 1 ;;
    esac
}

# ---- Create a test sandbox ----
# Usage: create_test_sandbox <platform> <vm_name> [port]
# Installs nodejs + npm. Returns 0 on success.
create_test_sandbox() {
    local platform="$1" vm_name="$2" port="${3:-}"
    case "$platform" in
        macos)
            limactl delete --force "$vm_name" 2>/dev/null || true
            limactl start --name "$vm_name" --tty=false template://alpine 2>/dev/null || return 1
            sandbox_exec "$platform" "$vm_name" "sudo apk update && sudo apk add --no-cache nodejs npm git curl bash ca-certificates" >/dev/null 2>&1
            ;;
        linux)
            podman rm -f "$vm_name" 2>/dev/null || true
            local port_arg=""
            [ -n "$port" ] && port_arg="-p 127.0.0.1:${port}:${port}"
            podman run -d --name "$vm_name" $port_arg alpine:latest sleep 7200 2>/dev/null || return 1
            sandbox_exec "$platform" "$vm_name" "apk update && apk add --no-cache nodejs npm git curl bash ca-certificates" >/dev/null 2>&1
            ;;
        *)
            return 1
            ;;
    esac
}

# ---- Destroy a test sandbox ----
destroy_test_sandbox() {
    local platform="$1" vm_name="$2"
    case "$platform" in
        macos)   limactl delete --force "$vm_name" 2>/dev/null || true ;;
        linux)   podman rm -f "$vm_name" 2>/dev/null || true ;;
        windows) wsl --unregister "$vm_name" 2>/dev/null || true ;;
    esac
}

# ---- Timestamp ----
now_sec() { date +%s; }
