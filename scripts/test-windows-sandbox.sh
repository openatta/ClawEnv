#!/bin/bash
# Windows Sandbox Test — WSL2 + Alpine (DEFERRED: requires non-VM Windows host)
#
# This test is a framework placeholder. WSL2 cannot run inside a virtual machine
# (no nested Hyper-V). It requires a physical Windows machine or bare-metal host.
#
# When ready to implement:
#   1. Ensure WSL2 is enabled: clawenv install --mode sandbox --step prereq
#   2. Create Alpine distro: clawenv install --mode sandbox --step create
#   3. Install claw inside WSL2: clawenv install --mode sandbox --step claw
#   4. Full lifecycle: start/stop/restart/exec/logs
set -uo pipefail

echo "========================================"
echo "  Windows Sandbox Test (WSL2 + Alpine)"
echo "  STATUS: DEFERRED — requires physical Windows host"
echo "========================================"
echo ""
echo "  WSL2 cannot run in a virtual machine (no nested Hyper-V)."
echo "  This test requires a non-VM Windows 11 environment."
echo ""
echo "  To run manually on a physical Windows machine:"
echo "    clawenv install --mode sandbox --name test-wsl --step prereq"
echo "    clawenv install --mode sandbox --name test-wsl --step create"
echo "    clawenv install --mode sandbox --name test-wsl --step claw"
echo "    clawenv install --mode sandbox --name test-wsl --port 3400 --step config"
echo "    clawenv install --mode sandbox --name test-wsl --port 3400 --step gateway"
echo "    clawenv status test-wsl"
echo "    clawenv exec \"node --version\" test-wsl"
echo "    clawenv uninstall --name test-wsl"
echo ""

echo "  RESULTS: 0 passed, 0 failed, ALL SKIPPED (deferred)"
exit 0
