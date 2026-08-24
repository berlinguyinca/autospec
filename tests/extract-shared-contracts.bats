#!/usr/bin/env bats
# tests/extract-shared-contracts.bats — Spec 3 child D (prompt-cache-reclaim):
# scripts/extract-shared-contracts.sh is a DETERMINISTIC scanner that greps
# signatures / file paths / names across child issue bodies and emits a
# `## Shared contracts` block listing every token that appears in ≥2 issues.
# It replaces the mechanical steps 1-2 of the former Phase 3.75 Tier-A pass.
#
# Contract:
#   extract-shared-contracts.sh <body1.md> <body2.md> ...   # bodies as args
#   extract-shared-contracts.sh --dir <dir>                  # all *.md in <dir>
# Output (stdout): a `## Shared contracts` markdown block. Exit 0 always when
# inputs are readable; exit 2 on usage error. Deterministic: same inputs ->
# byte-identical output (cross-issue items sorted).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    EXTRACT="$REPO_ROOT/scripts/extract-shared-contracts.sh"
    TMP="$(mktemp -d -t extract-shared.XXXXXX)"
}

teardown() {
    rm -rf "$TMP"
}

@test "script exists and is executable" {
    [ -x "$EXTRACT" ]
}

@test "usage error (no args) exits 2" {
    run "$EXTRACT"
    [ "$status" -eq 2 ]
}

@test "emits a ## Shared contracts heading" {
    printf '## Implementation outline\n- `scripts/foo.sh` — x\n' > "$TMP/a.md"
    printf '## Implementation outline\n- `scripts/foo.sh` — y\n' > "$TMP/b.md"
    run "$EXTRACT" "$TMP/a.md" "$TMP/b.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"## Shared contracts"* ]]
}

@test "a file path appearing in two issues is reported as shared" {
    printf 'edit `scripts/shared-lib.sh` here\n' > "$TMP/a.md"
    printf 'also touch `scripts/shared-lib.sh` there\n' > "$TMP/b.md"
    run "$EXTRACT" "$TMP/a.md" "$TMP/b.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"scripts/shared-lib.sh"* ]]
}

@test "a path in only one issue is NOT reported as shared" {
    printf 'edit `scripts/only-a.sh` here\n' > "$TMP/a.md"
    printf 'edit `scripts/only-b.sh` there\n' > "$TMP/b.md"
    run "$EXTRACT" "$TMP/a.md" "$TMP/b.md"
    [ "$status" -eq 0 ]
    [[ "$output" != *"only-a.sh"* ]]
    [[ "$output" != *"only-b.sh"* ]]
}

@test "a function signature shared across issues is reported" {
    printf 'defines `bundle_static_context()` contract\n' > "$TMP/a.md"
    printf 'calls `bundle_static_context()` downstream\n' > "$TMP/b.md"
    run "$EXTRACT" "$TMP/a.md" "$TMP/b.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"bundle_static_context()"* ]]
}

@test "an env-var-prefix / NAMING token shared across issues is reported" {
    printf 'reads AUTOSPEC_MAX_MEMORY_FILES env\n' > "$TMP/a.md"
    printf 'caps on AUTOSPEC_MAX_MEMORY_FILES too\n' > "$TMP/b.md"
    run "$EXTRACT" "$TMP/a.md" "$TMP/b.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"AUTOSPEC_MAX_MEMORY_FILES"* ]]
}

@test "deterministic: identical inputs produce byte-identical output" {
    printf 'shares `scripts/x.sh` and ZZZ_VAR and AAA_VAR\n' > "$TMP/a.md"
    printf 'shares `scripts/x.sh` and ZZZ_VAR and AAA_VAR\n' > "$TMP/b.md"
    run "$EXTRACT" "$TMP/a.md" "$TMP/b.md"
    out1="$output"
    run "$EXTRACT" "$TMP/a.md" "$TMP/b.md"
    [ "$out1" = "$output" ]
    # sorted: AAA_VAR must appear before ZZZ_VAR
    line_aaa="$(printf '%s\n' "$output" | grep -n 'AAA_VAR' | head -1 | cut -d: -f1)"
    line_zzz="$(printf '%s\n' "$output" | grep -n 'ZZZ_VAR' | head -1 | cut -d: -f1)"
    [ "$line_aaa" -lt "$line_zzz" ]
}

@test "--dir mode scans every *.md in the directory" {
    printf 'edit `scripts/dirmode.sh`\n' > "$TMP/one.md"
    printf 'edit `scripts/dirmode.sh`\n' > "$TMP/two.md"
    run "$EXTRACT" --dir "$TMP"
    [ "$status" -eq 0 ]
    [[ "$output" == *"scripts/dirmode.sh"* ]]
}

@test "unreadable / missing input exits 2" {
    run "$EXTRACT" "$TMP/does-not-exist.md"
    [ "$status" -eq 2 ]
}

@test "single issue (no cross-issue overlap) emits an empty/none contracts block" {
    printf 'edit `scripts/solo.sh` and SOLO_VAR\n' > "$TMP/a.md"
    run "$EXTRACT" "$TMP/a.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"## Shared contracts"* ]]
    [[ "$output" != *"scripts/solo.sh"* ]]
}

@test "block is wrapped in shared-contracts markers on the populated path" {
    # scripts/lint-issue.sh exempts generated metadata from the authored word
    # budget only BETWEEN these markers, and only when the heading sits inside
    # them. Emitting the block bare charges every word to the issue's budget.
    printf 'edit `scripts/shared.sh` and SHARED_VAR\n' > "$TMP/a.md"
    printf 'also `scripts/shared.sh` and SHARED_VAR\n' > "$TMP/b.md"
    run "$EXTRACT" "$TMP/a.md" "$TMP/b.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"<!-- autospec-shared-contracts:begin -->"* ]]
    [[ "$output" == *"<!-- autospec-shared-contracts:end -->"* ]]
    # The heading must fall inside the pair, not above it.
    printf '%s\n' "$output" > "$TMP/out.md"
    begin="$(grep -n 'autospec-shared-contracts:begin' "$TMP/out.md" | cut -d: -f1)"
    head="$(grep -n '^## Shared contracts' "$TMP/out.md" | cut -d: -f1)"
    end="$(grep -n 'autospec-shared-contracts:end' "$TMP/out.md" | cut -d: -f1)"
    [ "$begin" -lt "$head" ]
    [ "$head" -lt "$end" ]
}

@test "block is wrapped in shared-contracts markers on the no-overlap path" {
    # The early-return branch is a separate exit path and needs its own end marker;
    # an unbalanced pair makes strip_generated_metadata decline to strip at all.
    printf 'edit `scripts/solo.sh` and SOLO_VAR\n' > "$TMP/a.md"
    run "$EXTRACT" "$TMP/a.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"<!-- autospec-shared-contracts:begin -->"* ]]
    [[ "$output" == *"<!-- autospec-shared-contracts:end -->"* ]]
}
