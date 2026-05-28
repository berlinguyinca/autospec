#!/usr/bin/env bats
# tests/qa/test_finding_filter.bats
#
# Coverage for the post-cluster finding filter (issue #647):
#   1. Verdict with all-fresh `verified_at` → filter passes; output unchanged.
#   2. Verdict with stale `verified_at` and probe-resolved finding → finding
#      dropped, listed in verified_dropped[].
#   3. Verdict with missing `verified_at` + AUTOSPEC_QA_STRICT=1 → filter
#      exits non-zero, prints offender.
#   4. Verdict with missing `verified_at` + AUTOSPEC_QA_STRICT=0 → finding
#      moved to unverified_warnings[], exit 0.
#   5. qa-verify-finding.sh --emit-record produces a finding with the three
#      required fields (verified_at, verified_by, verify_category).
#
# Discipline: real fixtures, no helper-script mocking. The filter is
# exercised against fixture JSON files written to a tmpdir; HEAD is taken
# from this repo so verified_at is a real SHA.

setup() {
    SCRIPT="$BATS_TEST_DIRNAME/../../scripts/qa-finding-filter.sh"
    VERIFY="$BATS_TEST_DIRNAME/../../scripts/qa-verify-finding.sh"
    [ -x "$SCRIPT" ] || skip "qa-finding-filter.sh not yet installed"
    [ -x "$VERIFY" ] || skip "qa-verify-finding.sh not yet installed"
    TMPDIR_T="$(mktemp -d)"
    HEAD_SHA="$(git -C "$BATS_TEST_DIRNAME/../.." rev-parse HEAD)"
}

teardown() {
    if [ -n "${TMPDIR_T:-}" ]; then
        rm -rf "$TMPDIR_T"
    fi
    return 0
}

write_verdict() {
    # $1 = path, $2 = json body
    printf '%s\n' "$2" > "$1"
}

@test "filter passes verdict with all-fresh verified_at unchanged" {
    verdict="$TMPDIR_T/qa-verdict.json"
    write_verdict "$verdict" "$(cat <<EOF
{
  "findings": [
    {
      "category": "missing_function",
      "release_blocking": true,
      "status": "fail",
      "summary": "foo missing",
      "evidence": "git grep foo",
      "verified_at": "$HEAD_SHA",
      "verified_by": "qa-verify-finding.sh",
      "verify_category": "missing_function"
    }
  ]
}
EOF
)"
    run env AUTOSPEC_QA_STRICT=1 bash "$SCRIPT" --verdict "$verdict"
    [ "$status" -eq 0 ]
    # Verdict file should still contain the finding
    grep -q '"summary": "foo missing"' "$verdict"
}

@test "filter strict mode rejects findings missing verified_at" {
    verdict="$TMPDIR_T/qa-verdict.json"
    write_verdict "$verdict" '{
      "findings": [
        {
          "category": "missing_function",
          "release_blocking": true,
          "status": "fail",
          "summary": "unverified finding",
          "evidence": "blind"
        }
      ]
    }'
    run env AUTOSPEC_QA_STRICT=1 bash "$SCRIPT" --verdict "$verdict"
    [ "$status" -ne 0 ]
    [[ "$output" == *"unverified finding"* ]] || [[ "$output" == *"verify_first_violation"* ]]
}

@test "filter non-strict mode moves unverified to unverified_warnings" {
    verdict="$TMPDIR_T/qa-verdict.json"
    write_verdict "$verdict" '{
      "findings": [
        {
          "category": "missing_function",
          "release_blocking": true,
          "status": "fail",
          "summary": "blind blob",
          "evidence": "no probe"
        }
      ]
    }'
    run env AUTOSPEC_QA_STRICT=0 bash "$SCRIPT" --verdict "$verdict"
    [ "$status" -eq 0 ]
    grep -q '"unverified_warnings"' "$verdict"
    grep -q '"blind blob"' "$verdict"
}

@test "filter stale verified_at triggers re-probe and drops resolved" {
    verdict="$TMPDIR_T/qa-verdict.json"
    # Use a symbol guaranteed present in tracked files at HEAD — the bash
    # function `qa-verify-finding` is referenced literally in the verify
    # script's own header, which has been tracked since #643.
    write_verdict "$verdict" '{
      "findings": [
        {
          "category": "missing_function",
          "release_blocking": true,
          "status": "fail",
          "summary": "stale finding for present symbol",
          "evidence": "old probe",
          "verified_at": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
          "verified_by": "qa-verify-finding.sh",
          "verify_category": "missing_function",
          "verify_args": {"symbol": "qa-verify-finding"}
        }
      ]
    }'
    run env AUTOSPEC_QA_STRICT=1 bash "$SCRIPT" --verdict "$verdict"
    [ "$status" -eq 0 ]
    grep -q '"verified_dropped"' "$verdict"
    # The finding should be moved into verified_dropped[] (not findings[]).
    python3 -c "
import json,sys
d=json.load(open('$verdict'))
assert not any(f.get('summary')=='stale finding for present symbol' for f in d.get('findings',[])), 'still in findings'
assert any(f.get('summary')=='stale finding for present symbol' for f in d.get('verified_dropped',[])), 'not in verified_dropped'
"
}

@test "qa-verify-finding.sh --emit-record writes required fields" {
    out="$TMPDIR_T/finding.json"
    run bash "$VERIFY" --emit-record "$out" \
        --category missing_function \
        --symbol some_definitely_absent_token_xyz_$RANDOM \
        --summary "absent token" \
        --evidence "git grep returned 0 hits"
    [ "$status" -eq 0 ]
    [ -f "$out" ]
    grep -q '"verified_at"' "$out"
    grep -q '"verified_by"' "$out"
    grep -q '"verify_category": "missing_function"' "$out"
}
