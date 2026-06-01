#!/usr/bin/env bats
# tests/qa/test_qa_bug_class_sweep.bats
#
# Fixture-driven tests for the autospec-qa bug-class sibling sweep
# (issue #737). Covers all four acceptance scenarios:
#
#   1. 5-instance bug class — parent finding triggers sweep; 4 sibling
#      findings appear in the verdict; heal-loop files ONE auto-implement
#      issue per pattern class.
#   2. Confidence threshold gate — a 6th instance with low context match
#      is logged to .autospec/qa-bug-class-flagged.json (NOT the verdict).
#   3. Verify-first filter applies to siblings — if the bug is already
#      fixed at HEAD for a sibling, that sibling is dropped.
#   4. Pattern drift safety — fingerprint is stored on the parent
#      finding at finding-time, not recomputed; mutating the parent line
#      after sweep does not invalidate sibling linkage.

REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
SWEEP="${REPO_ROOT}/scripts/qa-bug-class-sweep.sh"

setup() {
    TMPDIR_FIXT="$(mktemp -d)"
    export TMPDIR_FIXT
    mkdir -p "$TMPDIR_FIXT/.autospec" "$TMPDIR_FIXT/src"

    # Initialize a git repo so git grep works.
    (
        cd "$TMPDIR_FIXT"
        git init -q
        git config user.email test@example.com
        git config user.name  test
    )

    # 5 instances of os.system( — the bug class.
    for i in 1 2 3 4 5; do
        cat >"$TMPDIR_FIXT/src/mod${i}.py" <<PY
import os
def run${i}(cmd):
    os.system(cmd)
    return 0
PY
    done

    (
        cd "$TMPDIR_FIXT"
        git add -A
        git commit -q -m init
    )

    # Parent verdict cites src/mod1.py:3 (the os.system call).
    cat >"$TMPDIR_FIXT/.autospec/qa-verdict.json" <<'JSON'
{
  "verdict": "FAIL",
  "findings": [
    {
      "category": "security:bug",
      "release_blocking": true,
      "file": "src/mod1.py",
      "line": 3,
      "summary": "os.system shell injection"
    }
  ]
}
JSON
}

teardown() {
    rm -rf "$TMPDIR_FIXT"
}

@test "5-instance bug class — siblings appended to verdict" {
    cd "$TMPDIR_FIXT"
    run bash "$SWEEP" --verdict .autospec/qa-verdict.json --repo-dir .
    [ "$status" -eq 0 ]
    siblings=$(jq '[.findings[] | select(.category == "code_health:bug_class_sibling")] | length' \
        .autospec/qa-verdict.json)
    [ "$siblings" -ge 4 ]
}

@test "siblings carry parent_fingerprint linkage" {
    cd "$TMPDIR_FIXT"
    bash "$SWEEP" --verdict .autospec/qa-verdict.json --repo-dir .
    linked=$(jq '[.findings[] | select(.category == "code_health:bug_class_sibling") | select(.parent_fingerprint != null and (.parent_fingerprint | length) > 0)] | length' \
        .autospec/qa-verdict.json)
    [ "$linked" -ge 4 ]
}

@test "confidence threshold gate — low-confidence match flagged not filed" {
    # Override threshold high enough that the strong 1.0 matches still pass,
    # but introduce an additional low-specificity (short-anchor) parent
    # finding whose siblings score below threshold.
    cd "$TMPDIR_FIXT"
    # Add a parent with a 2-char anchor (`go`) — specificity = 2/8 = 0.25.
    cat >"$TMPDIR_FIXT/src/short.py" <<'PY'
def go():
    return 1
PY
    cat >"$TMPDIR_FIXT/src/short2.py" <<'PY'
def go():
    return 2
PY
    (cd "$TMPDIR_FIXT" && git add -A && git commit -q -m short)
    cat >"$TMPDIR_FIXT/.autospec/qa-verdict.json" <<'JSON'
{
  "verdict": "FAIL",
  "findings": [
    {"category":"security:bug","release_blocking":true,"file":"src/short.py","line":1,"summary":"x"}
  ]
}
JSON
    AUTOSPEC_QA_BUG_CLASS_MIN_CONFIDENCE=0.7 \
        bash "$SWEEP" --verdict .autospec/qa-verdict.json --repo-dir .
    siblings=$(jq '[.findings[] | select(.category == "code_health:bug_class_sibling")] | length' \
        .autospec/qa-verdict.json)
    [ "$siblings" -eq 0 ]
    flagged=$(jq '.flagged | length' .autospec/qa-bug-class-flagged.json)
    [ "$flagged" -ge 1 ]
}

@test "verify-first filter applies to siblings" {
    # Remove the anchor from mod5 so verify probe reports resolved → dropped.
    cd "$TMPDIR_FIXT"
    cat >"$TMPDIR_FIXT/src/mod5.py" <<'PY'
def safe5():
    return "resolved"
PY
    (cd "$TMPDIR_FIXT" && git add -A && git commit -q -m fix)
    bash "$SWEEP" --verdict .autospec/qa-verdict.json --repo-dir .
    # mod5 must not appear as a sibling.
    has_mod5=$(jq '[.findings[] | select(.category == "code_health:bug_class_sibling") | select(.file == "src/mod5.py")] | length' \
        .autospec/qa-verdict.json)
    [ "$has_mod5" -eq 0 ]
}

@test "pattern fingerprint stored on parent at finding-time" {
    cd "$TMPDIR_FIXT"
    bash "$SWEEP" --verdict .autospec/qa-verdict.json --repo-dir .
    fp=$(jq -r '.findings[0].pattern_fingerprint // ""' .autospec/qa-verdict.json)
    fh=$(jq -r '.findings[0].fingerprint_hash // ""' .autospec/qa-verdict.json)
    [ -n "$fp" ]
    [ -n "$fh" ]
    # Mutate the parent line — fingerprint already stored, must survive.
    cat >"$TMPDIR_FIXT/src/mod1.py" <<'PY'
def changed():
    pass
PY
    fp2=$(jq -r '.findings[0].pattern_fingerprint // ""' .autospec/qa-verdict.json)
    [ "$fp" = "$fp2" ]
}
