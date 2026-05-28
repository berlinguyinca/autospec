#!/usr/bin/env bats
# tests/qa/test_incident_check.bats
#
# Coverage for scripts/qa-incident-check.sh (issue #659):
#
#   1. active incident + present, non-trivial, passing, mutation-failing test
#      → check passes (exit 0), no findings appended.
#   2. active incident + missing test file → exit 1 with regression_test
#      missing finding.
#   3. active incident + tautological test (`expect(true).toBe(true)`) →
#      exit 1 with non-trivially assertive finding.
#   4. active incident + test that passes even under the recorded mutation
#      (no mutation-proof entry / observed_result != failed_as_expected) →
#      exit 1 with mutation proof missing finding.
#   5. superseded incident → not enforced, no findings.
#   6. empty registry → no findings, no warnings, exit 0.

setup() {
    SCRIPT="$BATS_TEST_DIRNAME/../../scripts/qa-incident-check.sh"
    [ -x "$SCRIPT" ] || skip "qa-incident-check.sh not installed"
    command -v jq >/dev/null 2>&1 || skip "jq required"
    TMPDIR_T="$(mktemp -d)"
    REGISTRY="$TMPDIR_T/production-incidents.json"
    VERDICT="$TMPDIR_T/qa-verdict.json"
    MUT="$TMPDIR_T/mutation-proof.json"
    mkdir -p "$TMPDIR_T/tests/regression"
}

teardown() {
    [ -n "${TMPDIR_T:-}" ] && rm -rf "$TMPDIR_T"
    return 0
}

write_registry() {
    printf '%s\n' "$1" > "$REGISTRY"
}

write_mutation_proof() {
    printf '%s\n' "$1" > "$MUT"
}

active_entry() {
    # $1 = regression_test path (relative)
    cat <<EOF
[
  {
    "id": "INC-2026-05-28-test",
    "occurred_at": "2026-05-28T00:00:00Z",
    "failure_mode": "double charge on submit",
    "feature": "checkout",
    "regression_test": "$1",
    "mutation": "remove submitting guard",
    "added_by": "tester",
    "status": "active"
  }
]
EOF
}

mutation_proof_failed() {
    # $1 = regression_test path (relative)
    cat <<EOF
{
  "schema_version": 1,
  "freshness": {
    "repo_commit_sha": "abcdef1",
    "spec_file": "docs/specs/x.md",
    "spec_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    "environment": "local",
    "generated_at": "2026-05-28T00:00:00Z"
  },
  "mutations": [
    {
      "workflow_id": "checkout",
      "mutation_type": "request_payload",
      "test": { "path": "$1", "name": "double-charge" },
      "expected_failure": "test must fail",
      "observed_result": "failed_as_expected"
    }
  ]
}
EOF
}

@test "passes when active incident has a non-trivial test and mutation proof" {
    rt="tests/regression/inc-double.test.ts"
    mkdir -p "$TMPDIR_T/$(dirname "$rt")"
    cat > "$TMPDIR_T/$rt" <<'EOF'
test("blocks duplicate Stripe charge", () => {
    const result = submitCheckout({ submitting: true });
    expect(result.charged).toBe(false);
    expect(result.error).toMatch(/already submitting/);
});
EOF
    write_registry "$(active_entry "$rt")"
    write_mutation_proof "$(mutation_proof_failed "$rt")"

    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 0 ]
    findings_count="$(jq '.findings | length' "$VERDICT")"
    [ "$findings_count" -eq 0 ]
}

@test "fails with regression_test missing finding when test file absent" {
    rt="tests/regression/does-not-exist.test.ts"
    write_registry "$(active_entry "$rt")"

    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 1 ]
    summary="$(jq -r '.findings[0].summary' "$VERDICT")"
    [[ "$summary" == *"regression_test missing"* ]]
    blocking="$(jq -r '.findings[0].release_blocking' "$VERDICT")"
    [ "$blocking" = "true" ]
}

@test "fails with non-trivially-assertive finding for tautological test" {
    rt="tests/regression/inc-tauto.test.ts"
    mkdir -p "$TMPDIR_T/$(dirname "$rt")"
    cat > "$TMPDIR_T/$rt" <<'EOF'
test("tautological", () => {
    expect(true).toBe(true);
});
EOF
    write_registry "$(active_entry "$rt")"
    write_mutation_proof "$(mutation_proof_failed "$rt")"

    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 1 ]
    summary="$(jq -r '.findings[0].summary' "$VERDICT")"
    [[ "$summary" == *"not non-trivially assertive"* ]]
}

@test "fails when mutation proof is missing or did not fail" {
    rt="tests/regression/inc-no-mut.test.ts"
    mkdir -p "$TMPDIR_T/$(dirname "$rt")"
    cat > "$TMPDIR_T/$rt" <<'EOF'
test("real assertion but no mutation proof", () => {
    const result = submitCheckout({ submitting: true });
    expect(result.charged).toBe(false);
});
EOF
    write_registry "$(active_entry "$rt")"
    # No mutation-proof.json written at all.

    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 1 ]
    summary="$(jq -r '.findings[0].summary' "$VERDICT")"
    [[ "$summary" == *"mutation proof missing"* ]]
}

@test "superseded incident is not enforced and emits no finding" {
    rt="tests/regression/does-not-exist.test.ts"
    write_registry "$(cat <<EOF
[
  {
    "id": "INC-2026-05-28-super",
    "occurred_at": "2026-05-28T00:00:00Z",
    "failure_mode": "ancient bug",
    "feature": "legacy",
    "regression_test": "$rt",
    "mutation": "x",
    "status": "superseded",
    "superseded_by": "docs/specs/new.md#section"
  }
]
EOF
)"

    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 0 ]
    findings_count="$(jq '.findings | length' "$VERDICT")"
    [ "$findings_count" -eq 0 ]
}

@test "inline mutation_proof with mismatched test path does NOT satisfy coverage" {
    rt="tests/regression/inc-mismatch.test.ts"
    mkdir -p "$TMPDIR_T/$(dirname "$rt")"
    cat > "$TMPDIR_T/$rt" <<'EOF'
test("real", () => { expect(charge()).toBe(false); });
EOF
    # Inline proof references a DIFFERENT test path → must not count.
    write_registry "$(cat <<EOF
[
  {
    "id": "INC-mismatch",
    "occurred_at": "2026-05-28T00:00:00Z",
    "failure_mode": "x",
    "feature": "x",
    "regression_test": "$rt",
    "mutation": "x",
    "status": "active",
    "mutation_proof": {
      "test": { "path": "tests/regression/OTHER.test.ts" },
      "observed_result": "failed_as_expected"
    }
  }
]
EOF
)"
    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 1 ]
    summary="$(jq -r '.findings[0].summary' "$VERDICT")"
    [[ "$summary" == *"mutation proof missing"* ]]
}

@test "assert.strictEqual is recognized as a real assertion (not trivial)" {
    rt="tests/regression/inc-strict.test.ts"
    mkdir -p "$TMPDIR_T/$(dirname "$rt")"
    cat > "$TMPDIR_T/$rt" <<'EOF'
const assert = require("assert");
test("strictEqual is real", () => {
    assert.strictEqual(result.status, 200);
});
EOF
    write_registry "$(active_entry "$rt")"
    write_mutation_proof "$(mutation_proof_failed "$rt")"

    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 0 ]
    findings_count="$(jq '.findings | length' "$VERDICT")"
    [ "$findings_count" -eq 0 ]
}

@test "expect(false).toBe(false) is detected as tautological" {
    rt="tests/regression/inc-false.test.ts"
    mkdir -p "$TMPDIR_T/$(dirname "$rt")"
    cat > "$TMPDIR_T/$rt" <<'EOF'
test("inverse tautology", () => {
    expect(false).toBe(false);
});
EOF
    write_registry "$(active_entry "$rt")"
    write_mutation_proof "$(mutation_proof_failed "$rt")"

    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 1 ]
    summary="$(jq -r '.findings[0].summary' "$VERDICT")"
    [[ "$summary" == *"not non-trivially assertive"* ]]
}

@test "empty registry produces no findings and exits 0" {
    write_registry "[]"

    run bash "$SCRIPT" --registry "$REGISTRY" --verdict "$VERDICT" \
        --mutation-proof "$MUT" --repo-root "$TMPDIR_T"
    [ "$status" -eq 0 ]
    # Verdict file may not even be created in this no-op path; tolerate.
    if [ -f "$VERDICT" ]; then
        findings_count="$(jq '.findings | length' "$VERDICT")"
        [ "$findings_count" -eq 0 ]
    fi
}
