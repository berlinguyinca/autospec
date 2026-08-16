#!/usr/bin/env bats
# Tests for the autospec-run SKILL.md body split (Phase 1: end-of-run tail).
#
# The 140K body was split so the hot monitor loop stays lean: the end-of-run
# procedures (Phase 6 final report, Phase 5.5 gap remediation, Phase 5.6 repo
# quality audit, advisor escalation) moved to a reference file the body points
# to. These tests pin the invariants that keep the split safe:
#   - the reference file exists and carries the moved sections
#   - every trio member points to it (MUST-read pointers)
#   - the pinned `## Phase 5.5` heading survives in the body (structural gate)
#   - the detailed procedures did NOT stay duplicated in the body
#
# Purely read-only: no fixtures are mutated, so no teardown snapshot is needed.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    SKILL_DIR="$REPO_ROOT/skills/autospec-run"
    REF="$SKILL_DIR/references/end-of-run.md"
    BODY="$SKILL_DIR/SKILL.md"
    CODEX="$SKILL_DIR/codex/prompt.md"
    OPENCODE="$SKILL_DIR/opencode/agent.md"
}

# --- the reference file exists and is non-trivial ---
@test "end-of-run reference file exists and is non-empty" {
    [ -f "$REF" ]
    [ -s "$REF" ]
    # it must hold real procedure content, not just a stub
    [ "$(wc -l < "$REF" | tr -d '[:space:]')" -gt 100 ]
}

# --- the reference holds every moved section ---
@test "reference contains all four moved sections" {
    grep -q '^## Phase 6 — Final report' "$REF"
    grep -q '^## Phase 5.5 — End-of-run gap remediation' "$REF"
    grep -q '^## Phase 5.6 — Repo quality audit' "$REF"
    grep -q '^## Advisor escalation' "$REF"
}

# --- every trio member points to the reference ---
@test "all three trio members carry MUST-read pointers to the reference" {
    for f in "$BODY" "$CODEX" "$OPENCODE"; do
        [ -f "$f" ]
        # four pointers: Phase 6, Phase 5.5, Phase 5.6, advisor
        [ "$(grep -c 'references/end-of-run.md' "$f")" -eq 4 ]
    done
}

# --- the pinned heading survives in the body (structural gate dependency) ---
@test "pinned '## Phase 5.5' heading is preserved in every trio member" {
    for f in "$BODY" "$CODEX" "$OPENCODE"; do
        grep -q '^## Phase 5.5 — End-of-run gap remediation' "$f"
    done
}

# --- the call-point script references the contract gates require stay in the body ---
# check_repo_quality_audit_loop and check_autospec_gap_miner_contract assert these
# "call point" anchors are present in every autospec-run trio member, so the
# pointers name the scripts even though the full procedures moved to the reference.
@test "call-point script references stay in every trio member" {
    for f in "$BODY" "$CODEX" "$OPENCODE"; do
        grep -q 'repo-quality-audit.sh' "$f"
        grep -q 'autospec-gap-miner.sh' "$f"
    done
}

# --- the detailed procedures did not stay duplicated in the body ---
@test "detailed end-of-run procedures are out of the body" {
    for f in "$BODY" "$CODEX" "$OPENCODE"; do
        # these markers live only in the moved reference, never in the body
        ! grep -q 'gap-remediation-loop.sh' "$f"
        ! grep -q 'fab-completeness.sh' "$f"
        ! grep -q 'advisor-sweep-tick.sh' "$f"
    done
}

# --- the body still keeps the phase landmarks (headings) in order ---
@test "body keeps the four phase headings as landmarks" {
    for h in '## Phase 6 — Final report' \
             '## Phase 5.5 — End-of-run gap remediation' \
             '## Phase 5.6 — Repo quality audit' \
             '## Advisor escalation'; do
        grep -q "^$h" "$BODY"
    done
}
