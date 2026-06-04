#!/usr/bin/env bats
# tests/gap-remediation-hardening.bats — hardening tests for gap-remediation-loop.sh
#
# Covers acceptance criteria for #1036:
#   AC1: all-invalid gaps file → exit 3 + stderr contains "all" and "failed schema"
#   AC2: WARN lines name missing fields and gap_id when present
#   AC3: mixed-validity file → exit 0 + summary contains "dropped="
#   AC4: valid file → behaviour byte-unchanged (regression)
#   AC5: dropped=N in summary when some gaps dropped
#
# Run: bats tests/gap-remediation-hardening.bats
#
# NOTE: the LOOP path resolves to skills/autospec-shared/scripts/gap-remediation-loop.sh
# (the canonical location of the script). The issue spec says "tests/gap-remediation-hardening.bats"
# as the bats file; the script to test lives in skills/autospec-shared/scripts/.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LOOP="$REPO_ROOT/skills/autospec-shared/scripts/gap-remediation-loop.sh"
FIXTURES="$REPO_ROOT/skills/autospec-shared/tests/fixtures"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_STATE_DIR="$TEST_TMP"
    export AUTOSPEC_GAP_REPO="testorg/testrepo"

    # Stub gh: handles issue list, label create, issue create, issue edit.
    mkdir -p "$TEST_TMP/bin"
    export GH_CREATE_LOG="$TEST_TMP/gh-create.log"
    export GH_ISSUE_LIST_JSON="$TEST_TMP/open-issues.json"
    printf '[]\n' > "$GH_ISSUE_LIST_JSON"

    cat > "$TEST_TMP/bin/gh" <<'GHEOF'
#!/usr/bin/env bash
case "$*" in
  *"issue list"*)
    cat "$GH_ISSUE_LIST_JSON"
    exit 0 ;;
  *"label create"*)
    exit 0 ;;
  *"issue create"*)
    printf '%s\n' "$*" >> "$GH_CREATE_LOG"
    echo "https://github.com/testorg/testrepo/issues/999"
    exit 0 ;;
  *"issue edit"*)
    exit 0 ;;
  *"repo view"*)
    echo "testorg/testrepo"; exit 0 ;;
esac
exit 0
GHEOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
}

teardown() {
    rm -rf "$TEST_TMP"
}

# ── AC4: valid file — behaviour unchanged (regression) ────────────────────────

@test "AC4: valid file exits 0 with survivors=1 filed=1 (regression)" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-valid.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" == *"survivors=1"* ]]
    [[ "$output" == *"filed=1"* ]]
    [[ "$output" != *"dropped="* ]]
}

@test "AC4: valid file has no WARN lines in stderr" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-valid.json" --file
    [ "$status" -eq 0 ]
    [[ "$output" != *"WARN gap"*"failed schema"* ]]
}

# ── AC1: all-invalid file → exit 3 + error message ───────────────────────────

@test "AC1: all-invalid file exits 3" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-all-invalid.json" --file
    [ "$status" -eq 3 ]
}

@test "AC1: all-invalid file stderr contains 'all' and 'failed schema'" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-all-invalid.json" --file
    [[ "$output" == *"all"* ]]
    [[ "$output" == *"failed schema"* ]]
}

@test "AC1: all-invalid file error message names emit-gaps.sh" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-all-invalid.json" --file
    [[ "$output" == *"emit-gaps.sh"* ]]
}

@test "AC1: all-invalid file names the count of dropped gaps" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-all-invalid.json" --file
    # The fixture has 4 gaps all invalid; message should include "4"
    [[ "$output" == *"4"* ]]
}

# ── AC2: WARN lines name missing fields and gap_id ───────────────────────────

@test "AC2: mixed file WARN line names missing fields for each dropped gap" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-mixed.json" --file
    # The invalid gap (index 1) is missing gap_id, severity, file, line, title, body, dedupe_key
    # WARN should name at least some of those fields
    [[ "$output" == *"missing:"* ]]
}

@test "AC2: all-invalid WARN lines include gap_id when present in object" {
    # gap-all-invalid.json has no gap_id; WARN lines should say gap_id is missing
    run bash "$LOOP" --gaps "$FIXTURES/gap-all-invalid.json" --file
    [[ "$output" == *"gap_id"* ]]
}

# ── AC3 & AC5: mixed file → exit 0, summary has dropped= ─────────────────────

@test "AC3: mixed file exits 0" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-mixed.json" --file
    [ "$status" -eq 0 ]
}

@test "AC3: mixed file summary contains dropped=" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-mixed.json" --file
    [[ "$output" == *"dropped="* ]]
}

@test "AC5: mixed file dropped=1 (one of three gaps is invalid)" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-mixed.json" --file
    [[ "$output" == *"dropped=1"* ]]
}

@test "AC5: mixed file survivors=2 (two valid gaps survive)" {
    run bash "$LOOP" --gaps "$FIXTURES/gap-mixed.json" --file
    [[ "$output" == *"survivors=2"* ]]
}
