#!/bin/bash
# Windows SSH remote helpers for E2E tests.
#
# The test runner lives on macOS but drives a Windows ARM64 box (typically
# UTM VM) via SSH. Every clawcli call, file probe, and HTTP check is
# wrapped in `ssh` to the Windows host. Output is parsed on the Mac side
# (same jq + shell logic as macOS scenarios).
#
# Reads `.env` at repo root for WIN_HOST / WIN_USER / WIN_PROJECT.

# Load Windows SSH config once per run.
e2e_win_load_env() {
    local env_file="${E2E_REPO_ROOT}/.env"
    if [ ! -f "$env_file" ]; then
        echo "[win-remote] .env missing — need WIN_HOST/WIN_USER/WIN_PROJECT" >&2
        return 1
    fi
    export $(grep -v '^#' "$env_file" | grep -E '^(WIN_HOST|WIN_USER|WIN_PROJECT)=' | xargs)
    if [ -z "${WIN_HOST:-}" ] || [ -z "${WIN_USER:-}" ]; then
        echo "[win-remote] WIN_HOST or WIN_USER missing in .env" >&2
        return 1
    fi
    : "${WIN_PROJECT:=C:/Users/$WIN_USER/ClawEnv}"

    # ENV_PREFIX for every cmd.exe invocation over SSH.
    #
    # CLAWENV_HOME: isolate test state into %USERPROFILE%\Desktop\ClawEnv-e2e\
    # so we don't pollute the real %USERPROFILE%\.clawenv. Native clawcli
    # uses dirs::home_dir() on Windows (SHGetKnownFolderPath) — can't be
    # redirected via $HOME like on macOS, so CLAWENV_HOME is the only knob.
    #
    # PATH: DO NOT add `C:\Program Files\nodejs` or `C:\Program Files\Git\cmd`.
    # The whole point of `--mode native` is that ClawEnv installs its OWN
    # node + git into %CLAWENV_HOME%\node\ and %CLAWENV_HOME%\git\ — the
    # net-check probes must exercise THOSE, not the system ones. A prior
    # version of this prefix pushed both system dirs onto PATH, which
    # caused `npm install` to silently resolve to `C:\Program Files\nodejs\npm.cmd`
    # when ClawEnv-native wasn't installed — producing a fake "PASS" that
    # told us nothing. Keep .cargo\bin (harmless; for any rust-tooling
    # sub-invocations) but nothing else.
    # Suffix the Windows-side CLAWENV_HOME with E2E_HOME_SUFFIX (set by
    # run.sh --parallel or --home-suffix) so two concurrent Win
    # scenarios don't clobber each other's %CLAWENV_HOME%\node\. Empty
    # suffix (serial mode) falls through to the shared dir — fine
    # because only one scenario runs at a time.
    local clawenv_suffix=""
    if [ -n "${E2E_HOME_SUFFIX:-}" ]; then
        clawenv_suffix="-${E2E_HOME_SUFFIX}"
    fi
    export WIN_CLAWENV_HOME="C:\\Users\\$WIN_USER\\Desktop\\ClawEnv-e2e${clawenv_suffix}"
    export WIN_ENV_PREFIX='set PATH=%PATH%;C:\Users\'"$WIN_USER"'\.cargo\bin&&set CLAWENV_HOME='"$WIN_CLAWENV_HOME"'&&'

    # clawcli.exe lives at target/release/clawcli.exe under the project.
    # Windows path — use backslashes for cmd.exe line + forward slashes
    # for most of the code since we pass through cmd which tolerates both.
    export WIN_CLAWCLI="target\\release\\clawcli.exe"

    # Quick liveness check.
    if ! ssh -o ConnectTimeout=5 -o BatchMode=yes "$WIN_USER@$WIN_HOST" "echo ok" 2>/dev/null | grep -q ok; then
        echo "[win-remote] cannot SSH to $WIN_USER@$WIN_HOST (add your pubkey or check VM)" >&2
        return 1
    fi
    echo "[win-remote] SSH OK → $WIN_USER@$WIN_HOST ($WIN_PROJECT)" >&2
}

# Run a command on Windows, cd'd into project dir, with full ENV_PREFIX.
# Captures stdout verbatim.
win_exec() {
    ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
        "$WIN_ENV_PREFIX cd $WIN_PROJECT && $*" 2>&1
}

# Run clawcli.exe --json on Windows, parse events on Mac (same UX as
# the local `cli()` wrapper). stdout captured locally; stderr streams
# to the test log via SSH.
cli_win() {
    local log="${E2E_TEST_HOME:-/tmp}/clawenv-e2e.log"
    {
        echo "────────────────────────────────────────────────────"
        echo "[cli-win] $(date '+%H:%M:%S') → $*"
    } | tee -a "$log" >&2

    local err_file
    err_file=$(mktemp)
    # Stream live — same rationale as cli(). SSH respects line buffering
    # on remote stdout by default so we get real-time progress on long
    # installs. Windows clawcli.exe doesn't need stdbuf; cmd's `&&`
    # chain passes line-mode output through.
    ssh -o ConnectTimeout=10 "$WIN_USER@$WIN_HOST" \
        "$WIN_ENV_PREFIX cd $WIN_PROJECT && $WIN_CLAWCLI --json $*" 2>>"$log" | \
    while IFS= read -r line; do
        echo "$line" >> "$log"
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
        esac
    done
    local rc=${PIPESTATUS[0]}

    local last_error=""
    [ -s "$err_file" ] && last_error=$(cat "$err_file")
    rm -f "$err_file"

    if [ "$rc" -ne 0 ] || [ -n "$last_error" ]; then
        echo "[cli-win] FAILED rc=$rc err=${last_error:-<none>}" >&2
        return 1
    fi
    return 0
}

# HTTP check against Windows side's localhost. Curl from Mac doesn't
# reach Windows 127.0.0.1, so we run curl ON the Windows box.
expect_http_200_win() {
    local url="$1"
    local timeout="${2:-60}"
    local deadline=$(($(date +%s) + timeout))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        # cmd.exe treats bare `%` as variable-expansion trigger — pass the
        # `%{http_code}` curl format through unscathed by using a
        # PowerShell one-liner which doesn't have this quirk.
        local code
        code=$(win_exec "powershell -NoProfile -Command \"try { (Invoke-WebRequest -Uri '$url' -UseBasicParsing -TimeoutSec 3).StatusCode } catch { 0 }\"" 2>/dev/null | tail -1 | tr -d '\r\n ')
        if [ "$code" = "200" ] || [ "$code" = "301" ] || [ "$code" = "302" ]; then
            _ok "HTTP $code from $url (via Windows)"
            return 0
        fi
        sleep 2
    done
    _fail "no HTTP 200 from $url within ${timeout}s (via Windows)"
    return 1
}

# File existence check on Windows. Path is Windows-style or forward-slash
# (cmd handles both).
expect_file_win() {
    local path="$1"
    local size
    size=$(win_exec "for %I in (\"$path\") do @echo %~zI" 2>/dev/null | tail -1 | tr -d '\r\n')
    if [ -n "$size" ] && [ "$size" != "0" ] && [ "$size" != "" ]; then
        _ok "file exists: $path ($size bytes, Windows)"
        return 0
    fi
    _fail "file missing or empty: $path (Windows)"
    return 1
}

# Check Windows config entry.
expect_config_entry_win() {
    local name="$1"
    # findstr tolerant of CRLF. Reads from CLAWENV_HOME-isolated location.
    if win_exec "findstr /C:\"name = \\\"$name\\\"\" \"$WIN_CLAWENV_HOME\\config.toml\"" 2>/dev/null | grep -q "$name"; then
        _ok "Windows config has instance '$name'"
        return 0
    fi
    _fail "Windows config.toml has no instance '$name'"
    return 1
}

expect_no_config_entry_win() {
    local name="$1"
    if win_exec "findstr /C:\"name = \\\"$name\\\"\" \"$WIN_CLAWENV_HOME\\config.toml\"" 2>/dev/null | grep -q "$name"; then
        _fail "Windows config.toml still has '$name'"
        return 1
    fi
    _ok "Windows config has no instance '$name'"
    return 0
}
