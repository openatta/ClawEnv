#!/bin/bash
# macOS Import Test — Lima image export/import + Native bundle import
#
# Prerequisites:
#   - An existing sandbox instance (for Lima export test) OR --skip-lima
#   - tools/package-native.sh must be available (for bundle test)
#
# Usage:
#   bash scripts/test-macos-import.sh
#   bash scripts/test-macos-import.sh --skip-lima     # Skip Lima export/import
#   bash scripts/test-macos-import.sh --skip-bundle   # Skip native bundle
set -uo pipefail

SKIP_LIMA=false
SKIP_BUNDLE=false
for arg in "$@"; do
    case "$arg" in
        --skip-lima)   SKIP_LIMA=true;;
        --skip-bundle) SKIP_BUNDLE=true;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib-test.sh"

echo "========================================"
echo "  macOS Import Test"
echo "========================================"

cd "$SCRIPT_DIR/.."
cargo build -p clawenv-cli 2>&1 | tail -1
find_cli

# ================================================================
section "A. Native Bundle — Generate & Import"
# ================================================================

if $SKIP_BUNDLE; then
    skip "bundle generate (--skip-bundle)"
    skip "bundle import (--skip-bundle)"
    skip "bundle verify (--skip-bundle)"
    skip "bundle cleanup (--skip-bundle)"
else
    BUNDLE_DIR="./test-bundle-$$"
    BUNDLE_INSTANCE="mac-bundle-$$"
    BUNDLE_PORT=3500

    # Check if package-native.sh exists
    if [[ ! -f tools/package-native.sh ]]; then
        skip "bundle generate (package-native.sh not found)"
        skip "bundle import"
        skip "bundle verify"
        skip "bundle cleanup"
    else
        # Generate bundle
        TOTAL=$((TOTAL+1))
        echo "       Generating native bundle (5-10 min)..."
        BUNDLE_RC=0
        bash tools/package-native.sh latest "$BUNDLE_DIR" 2>&1 | tail -5 || BUNDLE_RC=$?
        BUNDLE_FILE=$(ls "$BUNDLE_DIR"/clawenv-native-*.tar.gz 2>/dev/null | head -1)

        if [[ $BUNDLE_RC -eq 0 ]] && [[ -n "$BUNDLE_FILE" ]]; then
            pass "bundle generate"

            # Import
            run install --mode native --name "$BUNDLE_INSTANCE" --image "$BUNDLE_FILE" --port "$BUNDLE_PORT"
            if [[ $RC -eq 0 ]]; then pass "bundle import"; else fail "bundle import" "$(echo "$OUT" | tail -3)"; fi

            # Verify
            TOTAL=$((TOTAL+1)); RC=0
            OUT=$("$CLI" --json exec "openclaw --version" "$BUNDLE_INSTANCE" 2>&1) || RC=$?
            if echo "$OUT" | grep -qi "openclaw\|claw"; then pass "bundle verify"; else fail "bundle verify" "$OUT"; fi

            # Cleanup
            run uninstall --name "$BUNDLE_INSTANCE"
            if [[ $RC -eq 0 ]]; then pass "bundle cleanup"; else fail "bundle cleanup" "$OUT"; fi
        else
            fail "bundle generate" "package-native.sh failed (rc=$BUNDLE_RC)"
            skip "bundle import"
            skip "bundle verify"
            skip "bundle cleanup"
        fi

        rm -rf "$BUNDLE_DIR" 2>/dev/null
    fi
fi

# ================================================================
section "B. Lima Image — Export & Import"
# ================================================================

if $SKIP_LIMA; then
    skip "lima export (--skip-lima)"
    skip "lima import (--skip-lima)"
    skip "lima verify (--skip-lima)"
    skip "lima cleanup (--skip-lima)"
else
    # Need an existing sandbox instance to export from
    EXISTING=$("$CLI" --json list 2>&1 | grep -o '"name":"[^"]*"' | grep -v "Native" | head -1 | sed 's/"name":"//;s/"//')

    if [[ -z "$EXISTING" ]]; then
        skip "lima export (no sandbox instance found)"
        skip "lima import"
        skip "lima verify"
        skip "lima cleanup"
    else
        EXPORT_DIR="./test-export-$$"
        IMPORT_INSTANCE="mac-import-$$"

        # Export
        TOTAL=$((TOTAL+1))
        echo "       Exporting instance '$EXISTING'..."
        EXPORT_RC=0
        "$CLI" export "$EXISTING" --output "$EXPORT_DIR" 2>&1 | tail -3 || EXPORT_RC=$?
        EXPORT_FILE=$(ls "$EXPORT_DIR"/*.tar.gz 2>/dev/null | head -1)

        if [[ $EXPORT_RC -eq 0 ]] && [[ -n "$EXPORT_FILE" ]]; then
            pass "lima export"

            # Import
            run import "$EXPORT_FILE" --name "$IMPORT_INSTANCE"
            if [[ $RC -eq 0 ]]; then pass "lima import"; else fail "lima import" "$OUT"; fi

            # Verify
            run start "$IMPORT_INSTANCE"
            if [[ $RC -eq 0 ]]; then
                TOTAL=$((TOTAL+1)); RC=0
                OUT=$("$CLI" --json exec "echo import-ok" "$IMPORT_INSTANCE" 2>&1) || RC=$?
                if echo "$OUT" | grep -q "import-ok"; then pass "lima verify"; else fail "lima verify" "$OUT"; fi
            else
                fail "lima verify" "start failed"
            fi

            # Cleanup
            run uninstall --name "$IMPORT_INSTANCE"
            if [[ $RC -eq 0 ]]; then pass "lima cleanup"; else fail "lima cleanup" "$OUT"; fi
        else
            fail "lima export" "export failed (rc=$EXPORT_RC)"
            skip "lima import"
            skip "lima verify"
            skip "lima cleanup"
        fi

        rm -rf "$EXPORT_DIR" 2>/dev/null
    fi
fi

summary
