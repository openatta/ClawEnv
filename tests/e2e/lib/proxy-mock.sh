#!/bin/bash
# Mini-proxy lifecycle helpers.
#
# e2e_proxy_start <listen_port> <upstream_host> <upstream_port>
#   Launches mini_proxy.py in background, waits for READY signal on
#   stdout. Sets E2E_PROXY_PID for later shutdown.
#
# e2e_proxy_stop
#   Kills the mini-proxy process.
#
# e2e_proxy_check_upstream <host> <port>
#   Returns 0 if the upstream is alive (TCP probe), 1 otherwise. Used
#   by run.sh to skip proxy scenarios gracefully when user's real
#   proxy isn't running.

e2e_proxy_start() {
    local listen_port="$1"
    local upstream_host="${2:-127.0.0.1}"
    local upstream_port="${3:-7890}"
    local listen_host="${4:-127.0.0.1}"

    local script="${E2E_REPO_ROOT}/tests/e2e/lib/mini_proxy.py"
    if [ ! -f "$script" ]; then
        echo "[proxy-mock] mini_proxy.py not found at $script" >&2
        return 1
    fi

    # Launch in background, capture output to log + read READY marker.
    local log="$E2E_TEST_HOME/mini-proxy.log"
    python3 "$script" \
        --listen-host "$listen_host" \
        --listen-port "$listen_port" \
        --upstream-host "$upstream_host" \
        --upstream-port "$upstream_port" \
        > "$log" 2>&1 &
    export E2E_PROXY_PID=$!

    # Wait up to 5s for READY line.
    local deadline=$(($(date +%s) + 5))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if grep -q "^READY$" "$log" 2>/dev/null; then
            echo "[proxy-mock] started pid=$E2E_PROXY_PID 127.0.0.1:$listen_port → $upstream_host:$upstream_port" >&2
            return 0
        fi
        sleep 0.2
    done

    echo "[proxy-mock] failed to start within 5s — tail:" >&2
    tail -20 "$log" >&2 2>/dev/null
    return 1
}

e2e_proxy_stop() {
    if [ -n "${E2E_PROXY_PID:-}" ]; then
        kill "$E2E_PROXY_PID" 2>/dev/null
        wait "$E2E_PROXY_PID" 2>/dev/null
        echo "[proxy-mock] stopped pid=$E2E_PROXY_PID" >&2
        unset E2E_PROXY_PID
    fi
}

# TCP probe: returns 0 if port is accepting connections, 1 otherwise.
# Used to decide whether to skip proxy scenarios.
e2e_proxy_check_upstream() {
    local host="$1"
    local port="$2"
    if command -v nc >/dev/null; then
        nc -z -G 2 "$host" "$port" 2>/dev/null
    else
        # Pure-bash /dev/tcp fallback.
        (exec 3<>/dev/tcp/"$host"/"$port") 2>/dev/null && exec 3>&- 3<&-
    fi
}
