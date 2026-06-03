#!/usr/bin/env bash
# tests/phase4/test_docs_drift_gate_regenerate.sh
# Verifies the autospec-run Phase 4 docs-drift-gate `regenerate` self-heal wiring
# (issue #922, spec §D6 row 1):
#   1. all three trio files carry the docs-drift-gate begin/end markers and the
#      pinned `regenerate` action wiring (lock-step content);
#   2. the embedded gate bash extracted from SKILL.md is well-formed (`bash -n`);
#   3. the gate invokes doc-orchestrator.mjs (scoped regen) AND verify-examples.mjs
#      AND commits `docs: regenerate` into the PR branch, and applies `docs:failed`
#      on the failure path (code-PR-never-blocked semantics).
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

fail() { echo "FAIL: $1" >&2; exit 1; }

SKILL="$SCRIPT_DIR/skills/autospec-run/SKILL.md"
CODEX="$SCRIPT_DIR/skills/autospec-run/codex/prompt.md"
OPENCODE="$SCRIPT_DIR/skills/autospec-run/opencode/agent.md"

# ── 1. Markers + pinned content present in all three trio files ───────────────

for f in "$SKILL" "$CODEX" "$OPENCODE"; do
    name="${f#"$SCRIPT_DIR"/}"
    grep -qF '<!-- docs-drift-gate:begin -->' "$f" \
        || fail "$name: missing docs-drift-gate:begin marker"
    grep -qF '<!-- docs-drift-gate:end -->' "$f" \
        || fail "$name: missing docs-drift-gate:end marker"
    # Pinned action name.
    grep -q 'regenerate' "$f" \
        || fail "$name: missing 'regenerate' self-heal action wiring"
    grep -q 'doc-orchestrator.mjs' "$f" \
        || fail "$name: gate must invoke doc-orchestrator.mjs (scoped regen)"
    grep -q 'verify-examples.mjs' "$f" \
        || fail "$name: gate must re-verify with verify-examples.mjs"
    grep -q 'docs:failed' "$f" \
        || fail "$name: gate must apply docs:failed on generation failure"
    grep -q 'docs: regenerate' "$f" \
        || fail "$name: gate must commit 'docs: regenerate <scopes>' into the PR"
done

# ── 2. Extract the gate bash from SKILL.md and bash -n it ─────────────────────
# The block lives inside a blockquote (`> ` prefix) and a ```bash fence between
# the docs-drift-gate markers. Strip the blockquote prefix, pull the fenced
# bash, substitute placeholders, and syntax-check.

extract_gate_bash() {
    awk '
        /docs-drift-gate:begin/ { in_gate=1 }
        /docs-drift-gate:end/   { in_gate=0 }
        in_gate {
            line=$0
            sub(/^>[[:space:]]?/, "", line)        # strip blockquote prefix
            if (line ~ /^[[:space:]]*```bash[[:space:]]*$/) { in_fence=1; next }
            if (line ~ /^[[:space:]]*```[[:space:]]*$/ && in_fence) { in_fence=0; next }
            if (in_fence) print line
        }
    ' "$SKILL"
}

GATE_BASH="$(extract_gate_bash)"
[ -n "$GATE_BASH" ] || fail "could not extract gate bash from SKILL.md"

# Substitute the <PR>/<ISSUE> placeholders so `<` is not parsed as redirection.
TMP="$(mktemp -t drift-gate-XXXXXX.sh)"
trap 'rm -f "$TMP"' EXIT
{
    echo 'set -eu'
    printf '%s\n' "$GATE_BASH" | sed -e 's/<PR>/999/g' -e 's/<ISSUE>/922/g'
} > "$TMP"

bash -n "$TMP" || fail "extracted gate bash is not well-formed (bash -n failed)"

echo "OK: docs-drift-gate regenerate self-heal wiring verified"
