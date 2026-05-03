#!/usr/bin/env bash
# tests/integration/test_guardian_flow.sh — end-to-end guardian flow test.
#
# Requires: GH_TOKEN set (live GitHub access)
# Skips cleanly with exit 0 when GH_TOKEN is unset (matches CI gate pattern).
#
# What it does:
#   1. Runs lint-implementation.sh against the fixture diff file (offline mode).
#   2. Asserts the findings buffer schema matches the documented format.
#   3. Checks exit codes are correct.

set -eu

[ -z "${GH_TOKEN:-}" ] && { echo "SKIP: GH_TOKEN not set — skipping guardian integration test"; exit 0; }

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LINT="$REPO_ROOT/scripts/lint-implementation.sh"
FIX="$REPO_ROOT/tests/fixtures/implementation-quality"
TMP_FINDINGS="$(mktemp -t guardian-integration.XXXXXX.md)"
trap 'rm -f "$TMP_FINDINGS"' EXIT INT TERM

echo "guardian-flow: running lint-implementation.sh against bad-secret.diff..."
set +e
bash "$LINT" --diff-file "$FIX/bad-secret.diff" > "$TMP_FINDINGS" 2>/dev/null
det_exit=$?
set -e

echo "guardian-flow: det_exit=$det_exit"

# Assert exit code is >= 1 (findings present)
if [ "$det_exit" -lt 1 ]; then
    echo "FAIL: expected det_exit >= 1 for bad-secret.diff, got $det_exit" >&2
    exit 1
fi

# Assert findings buffer has RULE_ID:<path>:<line>: <desc> schema lines
if ! grep -qE '^[A-Z_]+:[^:]+:[^:]+: .+' "$TMP_FINDINGS"; then
    echo "FAIL: findings buffer does not match schema RULE_ID:<path>:<line>: <desc>" >&2
    cat "$TMP_FINDINGS" >&2
    exit 1
fi

# Assert at least one SECURITY finding
if ! grep -q "^SECURITY:" "$TMP_FINDINGS"; then
    echo "FAIL: expected SECURITY finding in findings buffer" >&2
    cat "$TMP_FINDINGS" >&2
    exit 1
fi

echo "guardian-flow: PASS — findings buffer schema validated, SECURITY finding present"
echo "guardian-flow: findings:"
cat "$TMP_FINDINGS"
exit 0
