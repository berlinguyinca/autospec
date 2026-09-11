#!/usr/bin/env bats
# tests/lint-issue.bats — issue-body quality-gate rule engine coverage
# (issue #3203: vacuous acceptance criteria must be rejected)

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LINTER="$REPO_ROOT/scripts/lint-issue.sh"
FIXTURES="$REPO_ROOT/tests/fixtures/lint-issue"

@test "the new rule id appears in the documentation block" {
  run grep -F 'AC_VACUOUS' "$LINTER"
  [ "$status" -eq 0 ]
  run bash "$LINTER" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"AC_VACUOUS"* ]]
}

@test "an AC asserting 0 occurrences of a literal emits AC_VACUOUS" {
  run bash "$LINTER" "$FIXTURES/vacuous-zero-occurrence.md"
  [ "$status" -gt 0 ]
  [[ "$output" == *AC_VACUOUS* ]]
}

@test "an AC asserting absence of a literal emits AC_VACUOUS" {
  run bash "$LINTER" "$FIXTURES/vacuous-absence.md"
  [ "$status" -gt 0 ]
  [[ "$output" == *AC_VACUOUS* ]]
}

@test "an AC pairing absence with a positive assertion emits 0 findings" {
  run bash "$LINTER" "$FIXTURES/paired-positive.md"
  [ "$status" -eq 0 ]
  [[ -z "$output" ]]
}

@test "both known-vacuous fixtures are flagged" {
  flagged=0
  for fixture in vacuous-zero-occurrence.md vacuous-absence.md; do
    run bash "$LINTER" "$FIXTURES/$fixture"
    if [ "$status" -gt 0 ] && [[ "$output" == *AC_VACUOUS* ]]; then
      flagged=$((flagged + 1))
    fi
  done
  [ "$flagged" -eq 2 ]
}

# ── AS-DAG warning-level checks (issue #3831, rollout stage 1) ──
# Warnings are reported but never affect the exit code.

write_dag_body() {
  # $1 = target file, $2 = dependencies-section body, $3 = optional extra sections
  cat > "$1" <<EOF
## Goal
Add a \`scripts/validate-dag.sh\` that exits 0 on a clean dependency graph.

## Files to read first
- crates/autospec-core/src/lint/dag.rs

## Implementation outline
1. Add the validator under \`scripts/\`.

## Tests required
- shell

## Verification
### Primary smoke test

\`\`\`bash
bash scripts/validate-dag.sh
\`\`\`

## Acceptance criteria
- [ ] \`scripts/validate-dag.sh\` exits 0 on the fixture in tests/fixtures/dag/

## Dependencies
$2
$3
EOF
}

DAG_REASON_META='## Machine metadata

```
autospec:
  dependencies:
    hard:
      - issue: 42
        reason_code: consumes-new-interface
```
'

DAG_ARTIFACT_META='## Machine metadata

```
autospec:
  dependencies:
    hard:
      - issue: 42
        artifact: crates/autospec-core/src/lint/dag.rs
```
'

DAG_MISMATCH_META='## Machine metadata

```
autospec:
  dependencies:
    hard:
      - issue: 123
        reason_code: consumes-new-interface
```
'

@test "a reason-code-free dependency emits AS-DAG-001 as a warning (exit 0)" {
  body="$(mktemp)"
  write_dag_body "$body" 'Depends on issue #42' ''
  run bash "$LINTER" "$body"
  [ "$status" -eq 0 ]
  [[ "$output" == *"WARNING:AS-DAG-001"* ]]
  [[ "$output" == *"no reason code and no artifact named"* ]]
  rm -f "$body"
}

@test "a recognized reason_code in the metadata block suppresses AS-DAG-001" {
  body="$(mktemp)"
  write_dag_body "$body" 'Depends on issue #42' "$DAG_REASON_META"
  run bash "$LINTER" "$body"
  [ "$status" -eq 0 ]
  [[ -z "$output" ]]
  rm -f "$body"
}

@test "an artifact in the metadata block suppresses AS-DAG-001" {
  body="$(mktemp)"
  write_dag_body "$body" 'Depends on issue #42' "$DAG_ARTIFACT_META"
  run bash "$LINTER" "$body"
  [ "$status" -eq 0 ]
  [[ -z "$output" ]]
  rm -f "$body"
}

@test "metadata/markdown disagreement emits AS-DAG-009 as a warning (exit 0)" {
  body="$(mktemp)"
  write_dag_body "$body" 'Depends on issue #42' "$DAG_MISMATCH_META"
  run bash "$LINTER" "$body"
  [ "$status" -eq 0 ]
  [[ "$output" == *"AS-DAG-009"* ]]
  [[ "$output" == *"declares {123}"* ]]
  [[ "$output" == *"declares {42}"* ]]
  rm -f "$body"
}

# ── SCOPE_HEDGE checks (issue #4275) ─────────────────────────────────────────
# An agent implements the smallest thing the spec can be read as permitting;
# hedged wording in a scope section names an alternative acceptable outcome.

@test "the SCOPE_HEDGE rule id appears in the documentation block" {
  run grep -F 'SCOPE_HEDGE' "$LINTER"
  [ "$status" -eq 0 ]
  run bash "$LINTER" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"SCOPE_HEDGE"* ]]
}

@test "a hedged AC emits SCOPE_HEDGE citing the phrase" {
  run bash "$LINTER" "$FIXTURES/scope-hedge.md"
  [ "$status" -gt 0 ]
  [[ "$output" == *SCOPE_HEDGE* ]]
  [[ "$output" == *"for now"* ]]
}

@test "a hedged Implementation outline emits SCOPE_HEDGE citing the phrase" {
  run bash "$LINTER" "$FIXTURES/scope-hedge.md"
  [ "$status" -gt 0 ]
  [[ "$output" == *"## Implementation outline"* ]]
  [[ "$output" == *"conservative"* ]]
}

@test "a body with clean scope wording emits no SCOPE_HEDGE" {
  run bash "$LINTER" "$FIXTURES/paired-positive.md"
  [ "$status" -eq 0 ]
  [[ "$output" != *SCOPE_HEDGE* ]]
}

@test "AS-DAG warnings carry severity in --json output" {
  body="$(mktemp)"
  write_dag_body "$body" 'Depends on issue #42' "$DAG_MISMATCH_META"
  run bash "$LINTER" --json "$body"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"rule":"AS-DAG-001"'* ]]
  [[ "$output" == *'"rule":"AS-DAG-009"'* ]]
  [[ "$output" == *'"severity":"warning"'* ]]
  rm -f "$body"
}
