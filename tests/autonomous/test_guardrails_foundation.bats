#!/usr/bin/env bats
# tests/autonomous/test_guardrails_foundation.bats — parent autonomy guardrails
# foundation contracts for issue #1543. These tests exercise deterministic helper
# behavior only; child issues deepen each mechanism.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
GUARDRAILS="$REPO_ROOT/scripts/autonomous-guardrails.sh"
PREMERGE="$REPO_ROOT/scripts/autonomous-premerge-gate.sh"
RESILIENCE="$REPO_ROOT/scripts/autonomous-resilience.sh"

setup() {
    TMP="$(mktemp -d -t guardrails_foundation.XXXXXX)"
    STUB_DIR="$TMP/bin"
    mkdir -p "$STUB_DIR"
    export PATH="$STUB_DIR:$PATH"
    export AUTOSPEC_STATE_DIR="$TMP/state"
    mkdir -p "$AUTOSPEC_STATE_DIR"
    export AUTOSPEC_NOTIFY_SH="$STUB_DIR/notify.sh"
    export AUTOSPEC_GH_CMD="$STUB_DIR/gh"

    cat > "$STUB_DIR/autospec-qa" <<'STUB'
#!/bin/bash
printf 'autospec-qa: all checks passed\n'
STUB
    chmod +x "$STUB_DIR/autospec-qa"

    cat > "$STUB_DIR/autospec-secaudit" <<'STUB'
#!/bin/bash
printf 'autospec-secaudit: all checks passed\n'
STUB
    chmod +x "$STUB_DIR/autospec-secaudit"

    cat > "$STUB_DIR/notify.sh" <<'STUB'
#!/bin/bash
printf 'notify: %s %s\n' "${1:-}" "${2:-}" >> "$AUTOSPEC_STATE_DIR/notify.log"
STUB
    chmod +x "$STUB_DIR/notify.sh"

    GH_LOG="$TMP/gh.log"
    export GH_LOG
    cat > "$STUB_DIR/gh" <<'STUB'
#!/bin/bash
printf 'gh %s\n' "$*" >> "$GH_LOG"
case "$*" in
  *'commits/main/status'*) printf '{"state":"success","statuses":[]}\n' ;;
  *) printf '{}\n' ;;
esac
STUB
    chmod +x "$STUB_DIR/gh"
}

teardown() {
    rm -rf "$TMP"
}

@test "diff-guard blocks implementer edits to tests and eval harness files" {
    changed="$TMP/changed.txt"
    cat > "$changed" <<'FILES'
tests/autonomous/test_premerge_gate.bats
scripts/validate.sh
scripts/autonomous-premerge-gate.sh
FILES

    run bash "$GUARDRAILS" diff-guard --changed-files "$changed"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^DECISION:block$'
    printf '%s\n' "$output" | grep -q 'immutable_verifier_modified'
    printf '%s\n' "$output" | grep -q 'tests/autonomous/test_premerge_gate.bats'
    printf '%s\n' "$output" | grep -q 'scripts/validate.sh'
}

@test "premerge gate quarantines high-risk fenced blast-radius paths before QA" {
    changed="$TMP/changed.txt"
    printf 'scripts/autospec-autonomous.sh\nREADME.md\n' > "$changed"
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$PREMERGE" \
        --pr-branch feat/risky \
        --changed-files "$changed" \
        --repo berlinguyinca/autospec \
        --pr 1543

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^quarantine fenced_surface$'
    grep -q 'autospec:needs-human' "$GH_LOG"
}

@test "premerge gate writes provenance with rollback handle and passing evidence" {
    changed="$TMP/changed.txt"
    evidence="$TMP/gate-evidence.json"
    provenance="$TMP/provenance.json"
    printf 'docs/AUTONOMY-CHARTER.md\n' > "$changed"
    printf '{"suite":"validate","result":"pass"}\n' > "$evidence"
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$PREMERGE" \
        --pr-branch feat/docs-only \
        --repo berlinguyinca/autospec \
        --pr 1543 \
        --changed-files "$changed" \
        --gate-evidence "$evidence" \
        --rollback-handle origin/main@{before-1543} \
        --provenance-out "$provenance"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^merge-ok$'
    jq -e '.schema == "autospec.autonomous.merge_provenance.v1"' "$provenance"
    jq -e '.repo == "berlinguyinca/autospec" and .pr == 1543' "$provenance"
    jq -e '.rollback_handle == "origin/main@{before-1543}"' "$provenance"
    jq -e '.gate_evidence.result == "pass"' "$provenance"
    jq -e '.blast_radius.decision == "allow"' "$provenance"
}

@test "post-merge health halt triggers rollback command with provenance handle" {
    provenance="$TMP/provenance.json"
    cat > "$provenance" <<'JSON'
{
  "schema": "autospec.autonomous.merge_provenance.v1",
  "repo": "berlinguyinca/autospec",
  "pr": 1543,
  "rollback_handle": "rollback-ref-1543"
}
JSON
    ROLLBACK_LOG="$TMP/rollback.log"
    export ROLLBACK_LOG
    cat > "$STUB_DIR/rollback-stub" <<'STUB'
#!/bin/bash
printf 'rollback %s\n' "$*" >> "$ROLLBACK_LOG"
STUB
    chmod +x "$STUB_DIR/rollback-stub"
    cat > "$STUB_DIR/gh" <<'STUB'
#!/bin/bash
case "$*" in
  *'commits/main/status'*) printf '{"state":"failure","statuses":[{"state":"failure"}]}\n' ;;
  *) printf '{}\n' ;;
esac
STUB
    chmod +x "$STUB_DIR/gh"
    export AUTOSPEC_GH_CMD="$STUB_DIR/gh"
    export AUTOSPEC_ROLLBACK_CMD="$STUB_DIR/rollback-stub"

    run bash "$RESILIENCE" post-merge-health --repo berlinguyinca/autospec --provenance "$provenance"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q 'DECISION:rollback'
    grep -q 'rollback-ref-1543' "$ROLLBACK_LOG"
}
