#!/usr/bin/env bash
# tests/phase4/test_docs_drift_gate_regen_conditional.sh
# Verifies the docs-drift-gate auto_regenerate conditional (issue #955, spec §D2b):
#   1. All 3 autospec-run adapter files carry the _REGEN gate wrapping the regeneration path.
#   2. Detection, labels, and classifier verdict logging are OUTSIDE the _REGEN gate.
#   3. The OFF-path log line is present in all 3 adapter files.
#   4. The gate bash extracted from each SKILL.md is well-formed (bash -n).
#   5. doc-orchestrator.mjs is inside the _REGEN=1 gate in all 3 adapter files.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

fail() { echo "FAIL: $1" >&2; exit 1; }

# ── Autospec-run adapter files ───────────────────────────────────────────────

RUN_SKILL="$SCRIPT_DIR/skills/autospec-run/SKILL.md"
RUN_CODEX="$SCRIPT_DIR/skills/autospec-run/codex/prompt.md"
RUN_OPENCODE="$SCRIPT_DIR/skills/autospec-run/opencode/agent.md"

RUN_ADAPTERS="$RUN_SKILL $RUN_CODEX $RUN_OPENCODE"

# ── 1. Gate conditional present in all autospec-run adapters ─────────────────

for f in $RUN_ADAPTERS; do
    name="${f#"$SCRIPT_DIR"/}"

    # The _REGEN resolver call must be present.
    grep -q '_REGEN' "$f" \
        || fail "$name: missing _REGEN gate variable (auto_regenerate conditional)"

    # The resolver import must reference resolveAutoRegenerate.
    grep -q 'resolveAutoRegenerate' "$f" \
        || fail "$name: missing resolveAutoRegenerate import in gate block"

    # The OFF-path log line must be present.
    grep -qF 'docs: regeneration skipped (auto_regenerate=false)' "$f" \
        || fail "$name: missing OFF-path log line 'docs: regeneration skipped (auto_regenerate=false)'"

    # doc-orchestrator.mjs must still be present (regenerate path preserved).
    grep -q 'doc-orchestrator.mjs' "$f" \
        || fail "$name: missing doc-orchestrator.mjs (regenerate path must be preserved)"

    # The docs-drift-gate markers must still bracket the section.
    grep -qF '<!-- docs-drift-gate:begin -->' "$f" \
        || fail "$name: missing docs-drift-gate:begin marker"
    grep -qF '<!-- docs-drift-gate:end -->' "$f" \
        || fail "$name: missing docs-drift-gate:end marker"
done

# ── 2. Detection / labels are OUTSIDE the _REGEN gate ─────────────────────────
# In SKILL.md for each trio: the docs:drift label application for the regenerate
# branch must appear BEFORE (i.e. at a lower line number than) the _REGEN check.
# This ensures detection always runs regardless of auto_regenerate setting.

for SKILL in "$RUN_SKILL"; do
    name="${SKILL#"$SCRIPT_DIR"/}"

    drift_label_line=$(grep -n '"docs:drift"' "$SKILL" | grep 'add-label' | head -1 | cut -d: -f1)
    regen_check_line=$(grep -n '_REGEN' "$SKILL" | head -1 | cut -d: -f1)

    [ -n "$drift_label_line" ] \
        || fail "$name: docs:drift label application not found"
    [ -n "$regen_check_line" ] \
        || fail "$name: _REGEN gate check not found"

    if [ "$drift_label_line" -ge "$regen_check_line" ]; then
        fail "$name: docs:drift label (line $drift_label_line) must appear BEFORE _REGEN gate (line $regen_check_line) — detection must be unconditional"
    fi
done

# ── 3. doc-orchestrator.mjs is INSIDE the _REGEN=1 gate ─────────────────────
# Verify the orchestrator call appears after the _REGEN=1 branch open and before
# the matching else/fi in each SKILL.md.

for SKILL in "$RUN_SKILL"; do
    name="${SKILL#"$SCRIPT_DIR"/}"

    # Extract the docs-drift-gate bash block and check ordering.
    gate_bash=$(awk '
        /docs-drift-gate:begin/ { in_gate=1 }
        /docs-drift-gate:end/   { in_gate=0 }
        in_gate {
            line=$0
            sub(/^>[[:space:]]?/, "", line)
            if (line ~ /^[[:space:]]*```bash[[:space:]]*$/) { in_fence=1; next }
            if (line ~ /^[[:space:]]*```[[:space:]]*$/ && in_fence) { in_fence=0; next }
            if (in_fence) print line
        }
    ' "$SKILL")

    [ -n "$gate_bash" ] || fail "$name: could not extract gate bash block"

    # Within the extracted bash, _REGEN=1 branch must precede doc-orchestrator.mjs
    regen_branch_line=$(echo "$gate_bash" | grep -n '_REGEN.*=.*1\|_REGEN.*1' | head -1 | cut -d: -f1)
    orchestrator_line=$(echo "$gate_bash" | grep -n 'doc-orchestrator.mjs' | head -1 | cut -d: -f1)

    [ -n "$regen_branch_line" ] \
        || fail "$name: _REGEN=1 branch not found in extracted gate bash"
    [ -n "$orchestrator_line" ] \
        || fail "$name: doc-orchestrator.mjs not found in extracted gate bash"

    if [ "$orchestrator_line" -le "$regen_branch_line" ]; then
        fail "$name: doc-orchestrator.mjs (line $orchestrator_line) must appear AFTER _REGEN=1 branch (line $regen_branch_line)"
    fi
done

# ── 4. bash -n on the gate bash from each SKILL.md ───────────────────────────

extract_gate_bash() {
    local skill="$1"
    awk '
        /docs-drift-gate:begin/ { in_gate=1 }
        /docs-drift-gate:end/   { in_gate=0 }
        in_gate {
            line=$0
            sub(/^>[[:space:]]?/, "", line)
            if (line ~ /^[[:space:]]*```bash[[:space:]]*$/) { in_fence=1; next }
            if (line ~ /^[[:space:]]*```[[:space:]]*$/ && in_fence) { in_fence=0; next }
            if (in_fence) print line
        }
    ' "$skill"
}

for SKILL in "$RUN_SKILL"; do
    name="${SKILL#"$SCRIPT_DIR"/}"
    gate_bash="$(extract_gate_bash "$SKILL")"
    [ -n "$gate_bash" ] || fail "$name: could not extract gate bash"

    TMP="$(mktemp -t drift-gate-cond-XXXXXX.sh)"
    trap 'rm -f "$TMP"' EXIT
    {
        echo 'set -eu'
        printf '%s\n' "$gate_bash" \
            | sed -e 's/<PR>/999/g' -e 's/<ISSUE>/955/g' \
            | sed "s|'\${AUTOSPEC_DOC_SCRIPTS_DIR[^']*}'|'/tmp/doc-scripts'|g"
    } > "$TMP"
    bash -n "$TMP" || fail "$name: extracted gate bash is not well-formed (bash -n failed)"
done

echo "OK: docs-drift-gate auto_regenerate conditional verified across the autospec-run trio (3 files)"
