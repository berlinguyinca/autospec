#!/usr/bin/env bats
# tests/qa/test_exhaustiveness.bats
#
# Coverage for scripts/qa-exhaustiveness-check.sh (issue #660).
#
# Each fixture is self-contained: synthetic spec + synthetic artifact files,
# invoked through the explicit flag under test. Backwards-compat: no flags
# = no-op success (the script's behavior when called with no flags
# matches today's adapter behavior).

setup() {
    SCRIPT="$BATS_TEST_DIRNAME/../../scripts/qa-exhaustiveness-check.sh"
    [ -x "$SCRIPT" ] || skip "qa-exhaustiveness-check.sh not installed"
    command -v jq >/dev/null 2>&1 || skip "jq required"
    TMPDIR_T="$(mktemp -d)"
    VERDICT="$TMPDIR_T/qa-verdict.json"
    PROOF="$TMPDIR_T/proof-matrix.json"
    MUT="$TMPDIR_T/mutation-proof.json"
    CAN="$TMPDIR_T/canary-results.json"
    REL="$TMPDIR_T/reliability.yml"
    SPEC="$TMPDIR_T/spec.md"
}

teardown() {
    [ -n "${TMPDIR_T:-}" ] && rm -rf "$TMPDIR_T"
    return 0
}

@test "no flags → no-op success (backwards-compat)" {
    run bash "$SCRIPT" --verdict "$VERDICT"
    [ "$status" -eq 0 ]
    [ ! -f "$VERDICT" ]
}

@test "--every-spec-row passes when every AC has a non-NOT_TESTED row" {
    cat > "$SPEC" <<'EOF'
# Feature

## Acceptance

- user can log in with valid credentials
- user is shown an error on invalid password
EOF
    cat > "$PROOF" <<'EOF'
{
  "rows": [
    { "ac": "user can log in with valid credentials", "status": "PASS" },
    { "ac": "user is shown an error on invalid password", "status": "PARTIAL" }
  ]
}
EOF
    run bash "$SCRIPT" --every-spec-row \
        --verdict "$VERDICT" --proof-matrix "$PROOF" --spec "$SPEC"
    [ "$status" -eq 0 ]
}

@test "--every-spec-row fails with 5 ACs / 4 covered → missing AC named" {
    cat > "$SPEC" <<'EOF'
# Feature

## Acceptance

- ac one alpha
- ac two beta
- ac three gamma
- ac four delta
- ac five epsilon uncovered
EOF
    cat > "$PROOF" <<'EOF'
{
  "rows": [
    { "ac": "ac one alpha",   "status": "PASS" },
    { "ac": "ac two beta",    "status": "PASS" },
    { "ac": "ac three gamma", "status": "PARTIAL" },
    { "ac": "ac four delta",  "status": "PASS" }
  ]
}
EOF
    run bash "$SCRIPT" --every-spec-row \
        --verdict "$VERDICT" --proof-matrix "$PROOF" --spec "$SPEC"
    [ "$status" -eq 1 ]
    cat="$(jq -r '.findings[0].category' "$VERDICT")"
    [ "$cat" = "code_health:incomplete_traceability" ]
    summary="$(jq -r '.findings[0].summary' "$VERDICT")"
    [[ "$summary" == *"ac five epsilon uncovered"* ]]
    verdict="$(jq -r '.verdict' "$VERDICT")"
    [ "$verdict" = "PARTIAL" ]
}

@test "--every-spec-row treats NOT_TESTED rows as uncovered" {
    cat > "$SPEC" <<'EOF'
# Feature

## Acceptance

- only criterion
EOF
    cat > "$PROOF" <<'EOF'
{ "rows": [ { "ac": "only criterion", "status": "NOT_TESTED" } ] }
EOF
    run bash "$SCRIPT" --every-spec-row \
        --verdict "$VERDICT" --proof-matrix "$PROOF" --spec "$SPEC"
    [ "$status" -eq 1 ]
    cat="$(jq -r '.findings[0].category' "$VERDICT")"
    [ "$cat" = "code_health:incomplete_traceability" ]
}

@test "--mutation-each fails when 3 workflows / 2 covered" {
    cat > "$SPEC" <<'EOF'
# Feature

## Workflows

- checkout
- refund
- subscription renew
EOF
    cat > "$MUT" <<'EOF'
{
  "mutations": [
    { "workflow_id": "checkout", "observed_result": "failed_as_expected" },
    { "workflow_id": "refund",   "observed_result": "failed_as_expected" }
  ]
}
EOF
    run bash "$SCRIPT" --mutation-each \
        --verdict "$VERDICT" --mutation-proof "$MUT" --spec "$SPEC"
    [ "$status" -eq 1 ]
    cat="$(jq -r '.findings[0].category' "$VERDICT")"
    [ "$cat" = "code_health:incomplete_mutation_proof" ]
    summary="$(jq -r '.findings[0].summary' "$VERDICT")"
    [[ "$summary" == *"subscription renew"* ]]
}

@test "--no-mock-each fails when a backend feature has no canary entry" {
    cat > "$SPEC" <<'EOF'
# Feature

## Backend-backed Features

- billing/charge
- billing/refund
EOF
    cat > "$CAN" <<'EOF'
[
  { "feature": "billing/charge", "status": "PASS" }
]
EOF
    run bash "$SCRIPT" --no-mock-each \
        --verdict "$VERDICT" --canary "$CAN" --spec "$SPEC"
    [ "$status" -eq 1 ]
    cat="$(jq -r '.findings[0].category' "$VERDICT")"
    [ "$cat" = "code_health:incomplete_no_mock_coverage" ]
    summary="$(jq -r '.findings[0].summary' "$VERDICT")"
    [[ "$summary" == *"billing/refund"* ]]
}

@test "--no-mock-each honours reliability.yml exemptions" {
    cat > "$SPEC" <<'EOF'
# Feature

## Backend-backed Features

- billing/exempt-feature
EOF
    cat > "$CAN" <<'EOF'
[]
EOF
    cat > "$REL" <<'EOF'
no_mock_exempt:
  - billing/exempt-feature
EOF
    run bash "$SCRIPT" --no-mock-each \
        --verdict "$VERDICT" --canary "$CAN" --reliability "$REL" --spec "$SPEC"
    [ "$status" -eq 0 ]
}

@test "--exhaustive aggregates all three gates and records requested_flags" {
    cat > "$SPEC" <<'EOF'
# Feature

## Acceptance

- alpha
- beta

## Workflows

- alpha-flow

## Backend-backed Features

- alpha-service
EOF
    # Nothing covered → all three gates should fail.
    cat > "$PROOF" <<'EOF'
{ "rows": [] }
EOF
    cat > "$MUT" <<'EOF'
{ "mutations": [] }
EOF
    cat > "$CAN" <<'EOF'
[]
EOF
    run bash "$SCRIPT" --exhaustive \
        --verdict "$VERDICT" \
        --proof-matrix "$PROOF" \
        --mutation-proof "$MUT" \
        --canary "$CAN" \
        --spec "$SPEC"
    [ "$status" -eq 1 ]
    # At least 4 findings: 2 ACs + 1 workflow + 1 backend feature.
    count="$(jq '.findings | length' "$VERDICT")"
    [ "$count" -ge 4 ]
    # Flags persisted.
    has="$(jq -r '.requested_flags | index("--exhaustive")' "$VERDICT")"
    [ "$has" != "null" ]
    verdict="$(jq -r '.verdict' "$VERDICT")"
    [ "$verdict" = "PARTIAL" ]
}

@test "FAIL verdict is preserved (not upgraded) when gate fires" {
    printf '%s\n' '{"verdict":"FAIL","findings":[]}' > "$VERDICT"
    cat > "$SPEC" <<'EOF'
# Feature

## Acceptance

- only criterion
EOF
    cat > "$PROOF" <<'EOF'
{ "rows": [] }
EOF
    run bash "$SCRIPT" --every-spec-row \
        --verdict "$VERDICT" --proof-matrix "$PROOF" --spec "$SPEC"
    [ "$status" -eq 1 ]
    verdict="$(jq -r '.verdict' "$VERDICT")"
    [ "$verdict" = "FAIL" ]
}

@test "missing --spec when a flag is active → usage error (exit 2)" {
    run bash "$SCRIPT" --every-spec-row --verdict "$VERDICT"
    [ "$status" -eq 2 ]
}
