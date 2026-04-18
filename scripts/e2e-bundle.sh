#!/usr/bin/env bash
# e2e-bundle.sh — end-to-end bundle roundtrip test.
#
# Exercises the parts of the export/import pipeline that don't require a
# specific backend: manifest peek, wrap/unwrap, bail-on-missing-manifest.
# Runs against an already-installed instance (requires an OS+backend
# actually set up — this script does NOT install a VM, it reuses what's
# there).
#
# What it DOES verify (every CI run):
#   - clawcli export produces a tar.gz with clawenv-bundle.toml at root
#   - Manifest contains sane claw_type / sandbox_type / clawenv_version
#   - clawcli import rejects a tar.gz that has no manifest
#   - clawcli import accepts a valid bundle and records the right claw_type
#   - Exported+imported instance appears in `clawcli list`
#
# What it does NOT verify (requires a full backend — manual):
#   - The imported instance actually BOOTS (that's an OS-specific smoke
#     test in scripts/test-macos-sandbox.sh etc.)
#
# Usage:
#   ./scripts/e2e-bundle.sh                   # use instance 'default'
#   ./scripts/e2e-bundle.sh my-instance       # use a specific instance
#
# Exit codes:
#   0 — all assertions passed
#   1 — assertion failure (message printed to stderr)
#   2 — preconditions missing (no CLI, no instance)

set -euo pipefail

INSTANCE="${1:-default}"
TEST_TAG="$$"                                  # uniquify against concurrent runs
WORK_DIR="${TMPDIR:-/tmp}/clawenv-e2e-$TEST_TAG"
BUNDLE="$WORK_DIR/bundle.tar.gz"
FAKE_BUNDLE="$WORK_DIR/fake.tar.gz"
IMPORTED_NAME="e2e-imported-$TEST_TAG"

cd "$(dirname "$0")/.."

# ---- Locate the CLI ----
# Prefer release build, then debug, then PATH.
if [[ -x ./target/release/clawcli ]]; then
    CLI=./target/release/clawcli
elif [[ -x ./target/debug/clawcli ]]; then
    CLI=./target/debug/clawcli
elif command -v clawcli >/dev/null 2>&1; then
    CLI=clawcli
else
    echo "e2e: clawcli not found. Build it first: cargo build -p clawcli" >&2
    exit 2
fi
echo "e2e: using CLI at $CLI"

# ---- Precondition: instance exists ----
if ! "$CLI" list 2>/dev/null | grep -q "\"name\": \"$INSTANCE\""; then
    echo "e2e: instance '$INSTANCE' not found. Install one first or pass a name." >&2
    exit 2
fi
echo "e2e: using instance '$INSTANCE'"

# ---- Always clean up, even on failure ----
cleanup() {
    local rc=$?
    "$CLI" uninstall --name "$IMPORTED_NAME" >/dev/null 2>&1 || true
    rm -rf "$WORK_DIR"
    exit $rc
}
trap cleanup EXIT

mkdir -p "$WORK_DIR"

# ---- Test 1: export produces a well-formed bundle ----
echo ""
echo "[1/4] Exporting '$INSTANCE' → $BUNDLE"
"$CLI" export "$INSTANCE" --output "$BUNDLE"
[[ -f "$BUNDLE" ]] || { echo "FAIL: export did not produce $BUNDLE" >&2; exit 1; }

echo ""
echo "[2/4] Verifying manifest at archive root"
MANIFEST=$(tar -xzf "$BUNDLE" -O clawenv-bundle.toml 2>/dev/null || true)
if [[ -z "$MANIFEST" ]]; then
    echo "FAIL: bundle has no clawenv-bundle.toml at root" >&2
    exit 1
fi
echo "$MANIFEST" | head -8
# Field presence sanity — full schema enforcement is covered by manifest.rs
# unit tests; here we just make sure the producer didn't write garbage.
for field in schema_version clawenv_version claw_type sandbox_type; do
    echo "$MANIFEST" | grep -q "^${field} = " || {
        echo "FAIL: manifest missing required field '$field'" >&2
        exit 1
    }
done
echo "PASS: manifest schema looks sane"

# ---- Test 2: import bails on manifest-less tarball ----
echo ""
echo "[3/4] Verifying import rejects bundle without manifest"
(
    cd "$WORK_DIR"
    mkdir fake-src
    echo "not a bundle" > fake-src/placeholder.txt
    tar czf "$FAKE_BUNDLE" -C fake-src placeholder.txt
)
if "$CLI" import "$FAKE_BUNDLE" --name "should-not-exist-$TEST_TAG" >/dev/null 2>&1; then
    echo "FAIL: CLI imported a manifest-less tarball — should have bailed" >&2
    "$CLI" uninstall --name "should-not-exist-$TEST_TAG" >/dev/null 2>&1 || true
    exit 1
fi
echo "PASS: manifest-less bundle correctly rejected"

# ---- Test 3: real bundle imports back, with manifest-driven claw_type ----
echo ""
echo "[4/4] Importing bundle as '$IMPORTED_NAME' and verifying claw_type"
"$CLI" import "$BUNDLE" --name "$IMPORTED_NAME"

# Cross-check the imported instance against the original via the list
# output. The CLI emits one JSON object per instance — awk scans each
# object block, remembers the claw_type within it, and prints the value
# paired with whatever name the block finally declares. This is robust
# against field order (the JSON happens to list claw_type BEFORE name,
# which is why an early state machine that checked "name then claw_type"
# silently missed every match).
LIST_JSON=$("$CLI" list)
SRC_CLAW=$(printf '%s\n' "$LIST_JSON" | awk -v target="\"$INSTANCE\"" '
    /^\s*{/     { seen_claw = "" }
    /"claw_type":/ {
        n = split($0, a, "\"")
        seen_claw = a[4]
    }
    /"name":/ {
        if (index($0, target) > 0 && seen_claw != "") { print seen_claw; exit }
    }
')
DST_CLAW=$(printf '%s\n' "$LIST_JSON" | awk -v target="\"$IMPORTED_NAME\"" '
    /^\s*{/     { seen_claw = "" }
    /"claw_type":/ {
        n = split($0, a, "\"")
        seen_claw = a[4]
    }
    /"name":/ {
        if (index($0, target) > 0 && seen_claw != "") { print seen_claw; exit }
    }
')

if [[ -z "$DST_CLAW" ]]; then
    echo "FAIL: imported instance '$IMPORTED_NAME' not in list" >&2
    echo "--- list output ---" >&2
    printf '%s\n' "$LIST_JSON" >&2
    exit 1
fi
if [[ "$SRC_CLAW" != "$DST_CLAW" ]]; then
    echo "FAIL: imported claw_type '$DST_CLAW' doesn't match source '$SRC_CLAW'" >&2
    exit 1
fi
echo "PASS: imported instance recorded with claw_type=$DST_CLAW (from manifest)"

echo ""
echo "=========================================="
echo "ALL E2E BUNDLE TESTS PASSED"
echo "=========================================="
