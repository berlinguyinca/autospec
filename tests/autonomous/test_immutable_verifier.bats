#!/usr/bin/env bats
# tests/autonomous/test_immutable_verifier.bats — immutable verifier contracts
# for issue #1544. Fixtures simulate the deterministic gate inputs rather than
# invoking cargo-mutants or a real PR pipeline.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
GUARDRAILS="$REPO_ROOT/scripts/autonomous-guardrails.sh"
PREMERGE="$REPO_ROOT/scripts/autonomous-premerge-gate.sh"

setup() {
    TMP="$(mktemp -d -t immutable_verifier.XXXXXX)"
    STUB_DIR="$TMP/bin"
    mkdir -p "$STUB_DIR"
    export PATH="$STUB_DIR:$PATH"
    export AUTOSPEC_STATE_DIR="$TMP/state"
    mkdir -p "$AUTOSPEC_STATE_DIR"

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

@test "implementer-lane PR editing a test assertion is rejected by diff-guard" {
    changed="$TMP/changed.txt"
    printf 'tests/autonomous/test_guardrails_foundation.bats\n' > "$changed"

    run bash "$GUARDRAILS" diff-guard --lane implementer --changed-files "$changed"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^DECISION:block$'
    printf '%s\n' "$output" | grep -q '^REASON:immutable_verifier_modified$'
    printf '%s\n' "$output" | grep -q '^PATH:tests/autonomous/test_guardrails_foundation.bats$'
}

@test "mutation score below baseline blocks merge and lists surviving mutant" {
    changed="$TMP/changed.txt"
    baseline="$TMP/baseline.json"
    current="$TMP/current.json"
    printf 'skills/autospec-doc/scripts/doc-config.mjs\n' > "$changed"
    cat > "$baseline" <<'JSON'
{"score": 92, "mutants": [{"id": "M1", "status": "killed"}]}
JSON
    cat > "$current" <<'JSON'
{
  "score": 88,
  "surviving_mutants": [
    {"id": "M1", "file": "skills/autospec-doc/scripts/doc-config.mjs", "line": 42, "description": "changed >= to >"}
  ]
}
JSON

    run bash "$PREMERGE" \
        --pr-branch feat/mutation-regression \
        --changed-files "$changed" \
        --mutation-baseline "$baseline" \
        --mutation-current "$current" \
        --repo berlinguyinca/autospec \
        --pr 1544

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^DECISION:block$'
    printf '%s\n' "$output" | grep -q '^REASON:mutation_score_regression$'
    printf '%s\n' "$output" | grep -q '^MUTANT:M1:skills/autospec-doc/scripts/doc-config.mjs:42:changed >= to >$'
    printf '%s\n' "$output" | grep -q '^block mutation_score_regression$'
}

@test "immutable verifier bypass requires separate verifier lane and is tested" {
    changed="$TMP/changed.txt"
    printf 'tests/autonomous/test_guardrails_foundation.bats\n' > "$changed"

    run bash "$PREMERGE" \
        --pr-branch feat/verifier-updates-tests \
        --lane verifier \
        --changed-files "$changed" \
        --repo berlinguyinca/autospec \
        --pr 1544

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^merge-ok$'

    run bash "$PREMERGE" \
        --pr-branch feat/implementer-updates-tests \
        --lane implementer \
        --changed-files "$changed" \
        --repo berlinguyinca/autospec \
        --pr 1544

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q '^block immutable_verifier_modified$'
}
