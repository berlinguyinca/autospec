#!/usr/bin/env bats
# tests/unit/test_lint_doc_classification.bats — pins the is_doc_file contract:
# skills/*/prompts/*.md and skills/*/references/*.md are documentation, so a diff
# that adds a curated contract (referencing an existing CLI flag) plus a .sh with
# an env-var-shaped local var must NOT trip DOC_OUT_OF_SYNC.
#
# Lives in its own file (not test_lint_implementation.bats) because that file is
# already over the 600-line file-size ratchet and may not grow.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-implementation.sh"
    AUTOSPEC="$REPO_ROOT/target/debug/autospec"
    if [ ! -x "$AUTOSPEC" ]; then
        cargo build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" -p autospec-cli --bin autospec
    fi
    FIX="$REPO_ROOT/tests/fixtures/implementation-quality"
}

@test "lint-implementation: contract .md files count as docs (DOC_OUT_OF_SYNC suppressed)" {
    run bash "$LINT" --diff-file "$FIX/doc-contract-touched.diff"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -qE "^DOC_OUT_OF_SYNC:"
}

# 63 of this repo's 64 README files are not at the root, and subprojects keep
# their own docs/ tree. A flag documented in the subproject's own README used to
# trip DOC_OUT_OF_SYNC anyway, leaving no way to satisfy the gate except touching
# an unrelated root doc.
@test "lint-implementation: a nested README counts as a doc (DOC_OUT_OF_SYNC suppressed)" {
    run bash "$LINT" --diff-file "$FIX/nested-doc-touched.diff"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -qE "^DOC_OUT_OF_SYNC:"
}

# 826 of this repo's test files live outside the root tests/ tree, so anchoring
# is_test_file pointed the test-quality detectors at the smaller half of the
# codebase: MOCK_DB skipped those files entirely while TODO_LEFT treated them as
# production source. Both flip with the widened glob, which is why this test
# asserts one of each rather than only the absence of a finding.
@test "lint-implementation: a nested tests/ tree is treated as tests, not source" {
    run bash "$LINT" --vacuous-assertions --assertion-density \
        --diff-file "$FIX/nested-test-quality.diff"
    echo "$output" | grep -qE "^MOCK_DB:skills/autospec-shared/tests/unit/store_test\.py:"
    ! echo "$output" | grep -qE "^TODO_LEFT:"
}

# A .diff under tests/fixtures/ exists to CONTAIN a violation, so scanning its body
# reports the fixture as the violation -- which is how landing the fixture above
# was itself blocked. Data, not code.
@test "lint-implementation: a .diff fixture is data, not scannable source" {
    run bash "$LINT" --vacuous-assertions --assertion-density \
        --diff-file "$FIX/fixture-data-not-code.diff"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -qE "^(MOCK_DB|TODO_LEFT):"
}

# Prose describes a public surface, it cannot introduce one. A CHANGELOG entry
# mentioning a flag was read as that flag's introduction -- and CHANGELOG.md must
# NOT count as documentation for the requirement half, or every commit would
# satisfy the rule and the rule would be dead. Both halves are asserted here.
@test "lint-implementation: markdown is described-in, not scanned, and CHANGELOG earns no credit" {
    run bash "$LINT" --diff-file "$FIX/changelog-mentions-flag.diff"
    echo "$output" | grep -qE "^DOC_OUT_OF_SYNC:scripts/thing\.sh:"
    ! echo "$output" | grep -qE "^DOC_OUT_OF_SYNC:CHANGELOG\.md:"
}
