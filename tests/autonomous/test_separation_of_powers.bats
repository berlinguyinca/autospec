#!/usr/bin/env bats
# tests/autonomous/test_separation_of_powers.bats — separation-of-powers
# contracts for issue #1547. Fixtures simulate deterministic lane metadata so
# self-verification/self-approval is rejected before merge.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
GUARDRAILS="$REPO_ROOT/scripts/autonomous-guardrails.sh"
PREMERGE="$REPO_ROOT/scripts/autonomous-premerge-gate.sh"
REVIEWER_PROMPT="$REPO_ROOT/scripts/gen-reviewer-prompt.sh"

setup() {
    TMP="$(mktemp -d -t separation_powers.XXXXXX)"
    STUB_DIR="$TMP/bin"
    mkdir -p "$STUB_DIR"
    export PATH="$STUB_DIR:$PATH"
    export AUTOSPEC_STATE_DIR="$TMP/state"
    export AUTOSPEC_REPO_DIR="$TMP/repo"
    mkdir -p "$AUTOSPEC_STATE_DIR" "$AUTOSPEC_REPO_DIR/.autospec"

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

    GH_LOG="$TMP/gh.log"
    export GH_LOG
    cat > "$STUB_DIR/gh" <<'STUB'
#!/bin/bash
printf 'gh %s\n' "$*" >> "$GH_LOG"
printf '{}\n'
STUB
    chmod +x "$STUB_DIR/gh"
}

teardown() {
    rm -rf "$TMP"
}

write_lane_metadata() {
    local out="$1" author="$2" verifier="$3" approver="$4"
    cat > "$out" <<JSON
{
  "schema": "autospec.autonomous.lane_metadata.v1",
  "author": {"lane": "${author}", "agent_id": "${author}-agent"},
  "verifier": {"lane": "${verifier}", "agent_id": "${verifier}-agent"},
  "approver": {"lane": "${approver}", "agent_id": "${approver}-agent"},
  "verifier_prompt": {
    "mode": "adversarial",
    "context": "independent",
    "text": "Refute the change by default from an independent context. Do not trust author claims."
  }
}
JSON
}

@test "premerge gate rejects verification produced by the authoring lane" {
    changed="$TMP/changed.txt"
    metadata="$TMP/lane-metadata.json"
    printf 'scripts/autonomous-premerge-gate.sh\n' > "$changed"
    write_lane_metadata "$metadata" "implementer-a" "implementer-a" "approver-a"
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$PREMERGE" \
        --pr-branch feat/self-verification \
        --changed-files "$changed" \
        --lane verifier \
        --lane-metadata "$metadata" \
        --repo berlinguyinca/autospec \
        --pr 1547

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^DECISION:block$'
    printf '%s\n' "$output" | grep -q '^REASON:separation_of_powers_violation$'
    printf '%s\n' "$output" | grep -q '^VIOLATION:author_verifier_overlap$'
    printf '%s\n' "$output" | grep -q '^block separation_of_powers$'
}

@test "separation-of-powers guard rejects non-adversarial verifier prompts" {
    metadata="$TMP/lane-metadata.json"
    write_lane_metadata "$metadata" "implementer-a" "verifier-a" "approver-a"
    jq '.verifier_prompt.mode = "confirmatory" | .verifier_prompt.text = "Confirm the author claims."' \
        "$metadata" > "$metadata.tmp"
    mv "$metadata.tmp" "$metadata"

    run bash "$GUARDRAILS" separation-of-powers --lane-metadata "$metadata"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^DECISION:block$'
    printf '%s\n' "$output" | grep -q '^VIOLATION:verifier_prompt_not_adversarial$'
}

@test "passing premerge provenance records auditable author verifier approver lanes" {
    changed="$TMP/changed.txt"
    evidence="$TMP/gate-evidence.json"
    provenance="$TMP/provenance.json"
    metadata="$TMP/lane-metadata.json"
    printf 'docs/AUTONOMY-CHARTER.md\n' > "$changed"
    printf '{"suite":"validate","result":"pass"}\n' > "$evidence"
    write_lane_metadata "$metadata" "implementer-a" "verifier-a" "approver-a"
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$PREMERGE" \
        --pr-branch feat/separation-powers \
        --repo berlinguyinca/autospec \
        --pr 1547 \
        --changed-files "$changed" \
        --gate-evidence "$evidence" \
        --rollback-handle origin/main@{before-1547} \
        --lane verifier \
        --lane-metadata "$metadata" \
        --provenance-out "$provenance"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^merge-ok$'
    jq -e '.lane_metadata.author.lane == "implementer-a"' "$provenance"
    jq -e '.lane_metadata.verifier.lane == "verifier-a"' "$provenance"
    jq -e '.lane_metadata.approver.lane == "approver-a"' "$provenance"
    jq -e '.separation_of_powers.decision == "allow"' "$provenance"
}

@test "reviewer prompt is adversarial and explicitly independent of author context" {
    diff_file="$TMP/pr.diff"
    printf 'diff --git a/README.md b/README.md\n' > "$diff_file"

    run bash "$REVIEWER_PROMPT" --pr-diff "$diff_file" --repo berlinguyinca/autospec

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -qi 'refute.*default\|adversarial'
    printf '%s\n' "$output" | grep -qi 'independent.*author context\|author context.*independent'
}
