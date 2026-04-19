#!/bin/bash
# clawcli wrapper with --json parsing + test-friendly logging.
#
# Usage:
#   cli install --mode sandbox --name foo --port 10200
#   cli start foo
#
# Streams CliEvent JSON to both stderr (human summary) and the test log
# file. Returns 0 on Complete, non-zero on Error. Parses structured
# events via jq — hard-depend on jq.

# Resolve clawcli binary. Prefer release build in target/, fall back to
# whatever's on $PATH (lets CI point at a cached binary).
e2e_cli_bin() {
    local repo_root="${E2E_REPO_ROOT:-$(pwd)}"
    if [ -x "$repo_root/target/release/clawcli" ]; then
        echo "$repo_root/target/release/clawcli"
    elif [ -x "$repo_root/target/debug/clawcli" ]; then
        echo "$repo_root/target/debug/clawcli"
    else
        command -v clawcli 2>/dev/null || echo "clawcli"
    fi
}

# Main clawcli wrapper. Args passed through verbatim. Stdout captured,
# events streamed. Return code mirrors clawcli.
cli() {
    local bin
    bin=$(e2e_cli_bin)
    local log="${E2E_TEST_HOME:-/tmp}/clawenv-e2e.log"

    # Header in log + stderr for context.
    {
        echo "────────────────────────────────────────────────────"
        echo "[cli] $(date '+%H:%M:%S') → $*"
    } | tee -a "$log" >&2

    # Stream clawcli output to stderr live (progress visible during
    # multi-minute installs) AND record everything to the log. Capture
    # the last error via a tmp file so it survives the pipe subshell.
    local err_file
    err_file=$(mktemp)

    # `stdbuf -oL` line-buffers clawcli's stdout so each JSON event
    # comes through immediately rather than waiting for the 4KB OS
    # buffer to fill. Falls back gracefully if stdbuf isn't present
    # (unlikely on macOS with coreutils; bsd `unbuffer` covers it).
    local unbuf
    if command -v stdbuf >/dev/null; then
        unbuf=(stdbuf -oL -eL)
    else
        unbuf=()
    fi

    "${unbuf[@]}" "$bin" --json "$@" 2>>"$log" | \
    while IFS= read -r line; do
        echo "$line" >> "$log"
        if ! command -v jq >/dev/null; then
            echo "  (jq not installed; raw: $line)" >&2
            continue
        fi
        local kind
        kind=$(echo "$line" | jq -r '.type // "?"' 2>/dev/null)
        case "$kind" in
            progress)
                local pct msg
                pct=$(echo "$line" | jq -r '.percent // 0')
                msg=$(echo "$line" | jq -r '.message // ""')
                printf "  [%3s%%] %s\n" "$pct" "$msg" >&2
                ;;
            info)
                echo "  [info] $(echo "$line" | jq -r '.message // ""')" >&2
                ;;
            complete)
                echo "  [done] $(echo "$line" | jq -r '.message // ""')" >&2
                ;;
            error)
                local em
                em=$(echo "$line" | jq -r '.message // "unknown"')
                echo "$em" > "$err_file"
                echo "  [ERR!] $em" >&2
                ;;
            data)
                if [ "${E2E_VERBOSE:-0}" = "1" ]; then
                    echo "  [data] $(echo "$line" | jq -c '.data')" >&2
                fi
                ;;
        esac
    done
    local rc=${PIPESTATUS[0]}

    local last_error=""
    [ -s "$err_file" ] && last_error=$(cat "$err_file")
    rm -f "$err_file"

    if [ "$rc" -ne 0 ] || [ -n "$last_error" ]; then
        echo "[cli] FAILED rc=$rc err=${last_error:-<none>}" >&2
        return 1
    fi
    return 0
}

# Extract one field from the last `data` event of a command. Used when
# we need to read back config info (e.g. gateway_port).
cli_get_data() {
    local field="$1"
    shift
    local bin
    bin=$(e2e_cli_bin)
    "$bin" --json "$@" 2>/dev/null | \
        jq -r "select(.type==\"data\") | .data.$field" | tail -1
}
