#!/bin/bash
# Test Bridge Server API from inside the sandbox VM
# Usage: bash scripts/test-bridge.sh

BRIDGE_PORT=${1:-3100}
BASE="http://host.lima.internal:${BRIDGE_PORT}"
# For Podman: BASE="http://host.containers.internal:${BRIDGE_PORT}"

echo "=== Testing Bridge Server at ${BASE} ==="

echo "--- Health ---"
curl -s "${BASE}/api/health" | python3 -m json.tool 2>/dev/null || echo "FAILED"

echo "--- Permissions ---"
curl -s "${BASE}/api/permissions" | python3 -m json.tool 2>/dev/null || echo "FAILED"

echo "--- File Read ---"
curl -s -X POST "${BASE}/api/file/read" -H "Content-Type: application/json" \
  -d '{"path":"~/Documents/test.txt"}' | head -20

echo "--- File List ---"
curl -s -X POST "${BASE}/api/file/list" -H "Content-Type: application/json" \
  -d '{"path":"~"}' | python3 -m json.tool 2>/dev/null | head -20

echo "--- Exec ---"
curl -s -X POST "${BASE}/api/exec" -H "Content-Type: application/json" \
  -d '{"command":"echo","args":["hello from bridge"]}' | python3 -m json.tool 2>/dev/null

echo ""
echo "=== Tests complete ==="
