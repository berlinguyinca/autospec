#!/usr/bin/env bats
# Tests for the autospec SKILL.md Phase 4 cold-tail split (monitor recovery).
#
# The Phase 4 monitor pseudocode's cold tail (reviewer lens, reuse-BLOCK refute
# pass, verdict handling, Steps 8-11 SUCCESS/FAILURE/Cleanup/Report, monitor hard
# rules) moved to a reference file the body points to. These tests pin the
# invariants that keep the split safe:
#   - the reference file exists and carries the moved procedures
#   - every trio member points to it (MUST-read pointer)
#   - the detailed procedures did NOT stay duplicated in the body
#
# Purely read-only: no fixtures are mutated, so no teardown snapshot is needed.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    SKILL_DIR="$REPO_ROOT/skills/autospec"
    REF="$SKILL_DIR/references/monitor-recovery.md"
    BODY="$SKILL_DIR/SKILL.md"
    CODEX="$SKILL_DIR/codex/prompt.md"
    OPENCODE="$SKILL_DIR/opencode/agent.md"
}

# --- the reference file exists and is non-trivial ---
@test "monitor-recovery reference file exists and is non-empty" {
    [ -f "$REF" ]
    [ -s "$REF" ]
    # it must hold real procedure content, not just a stub
    [ "$(wc -l < "$REF" | tr -d '[:space:]')" -gt 100 ]
}

# --- the reference holds the moved cold-tail procedures ---
@test "reference contains the moved cold-tail markers" {
    grep -q 'data-scope invariant lens' "$REF"
    grep -q 'Regression gap-check' "$REF"
    grep -q 'reuse-BLOCK refute pass' "$REF"
    grep -q 'verify-voter-vendor.sh' "$REF"
    grep -q 'interrogation-ledger.sh' "$REF"
    grep -q 'Final output when shutdown' "$REF"
}

# --- every trio member points to the reference ---
@test "all three trio members carry a MUST-read pointer to the reference" {
    for f in "$BODY" "$CODEX" "$OPENCODE"; do
        [ -f "$f" ]
        [ "$(grep -c 'references/monitor-recovery.md' "$f")" -eq 1 ]
    done
}

# --- the detailed procedures did not stay duplicated in the body ---
@test "detailed cold-tail procedures are out of the body" {
    for f in "$BODY" "$CODEX" "$OPENCODE"; do
        # these markers live only in the moved reference, never in the body
        ! grep -q 'data-scope invariant lens' "$f"
        ! grep -q 'Final output when shutdown' "$f"
        ! grep -q 'verify-voter-vendor.sh' "$f"
        ! grep -q 'interrogation-ledger.sh' "$f"
    done
}
