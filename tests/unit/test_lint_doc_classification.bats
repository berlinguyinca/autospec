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
