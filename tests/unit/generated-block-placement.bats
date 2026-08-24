#!/usr/bin/env bats
# tests/unit/generated-block-placement.bats
#
# scripts/lint-issue.sh exempts generated metadata from the 400-word authored
# budget, but the exemption is line-bounded: only lines between a family's begin
# and end markers are dropped before counting.
#
# A template or emitter that puts its heading OUTSIDE the markers therefore leaks
# that heading into the authored count. Phase 3 issues routinely land at 380-399
# words, so the leak alone is enough to flag a whole batch BODY_TOO_LONG the
# moment Phase 3.5 patches it.
#
# The checker is a standalone script so it can be run directly:
#   python3 tests/lib/check_generated_blocks.py . templates
#   python3 tests/lib/check_generated_blocks.py . emitters

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    CHECK="$REPO_ROOT/tests/lib/check_generated_blocks.py"
}

@test "skills: every generated heading sits inside its marker pair" {
    run python3 "$CHECK" "$REPO_ROOT" templates
    [ "$status" -eq 0 ]
}

@test "emitters: the begin marker is written before the heading" {
    # Presence of the marker strings is not sufficient -- the original defect was a
    # marker in the wrong PLACE, not a missing one. This asserts source order.
    run python3 "$CHECK" "$REPO_ROOT" emitters
    [ "$status" -eq 0 ]
}
