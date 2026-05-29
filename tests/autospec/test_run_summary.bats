#!/usr/bin/env bats
# tests/autospec/test_run_summary.bats
#
# Cover the canonical .autospec/run-summary.md helper (issue #683).
# - Helper produces the canonical schema (all required sections present).
# - `## Next steps` is always present, with `- (none — converged)` fallback
#   when no next-steps content is provided.
# - `/autospec-refine --continue` can harvest next-steps content from a
#   fresh run-summary produced by the helper.

setup() {
    HELPER="${BATS_TEST_DIRNAME}/../../scripts/autospec-write-run-summary.sh"
    TMP_DIR="$(mktemp -d)"
    OUT="$TMP_DIR/run-summary.md"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "helper produces canonical schema with all required sections" {
    run bash "$HELPER" \
        --repo owner/repo \
        --prs "#1234 fix: bug,#1235 feat: thing" \
        --issues "#1230 task" \
        --failures "" \
        --elapsed "01:23" \
        --output "$OUT" \
        --sha abc1234 \
        --branch main
    [ "$status" -eq 0 ]
    [ -f "$OUT" ]
    # Required schema sections
    grep -q '^# autospec run summary — ' "$OUT"
    grep -q '^- HEAD SHA: abc1234' "$OUT"
    grep -q '^- Branch: main' "$OUT"
    grep -q '^- Total PRs merged: 2' "$OUT"
    grep -q '^- Issues closed: 1' "$OUT"
    grep -q '^- Elapsed: 01:23' "$OUT"
    grep -q '^## PRs merged' "$OUT"
    grep -q '^## Issues closed' "$OUT"
    grep -q '^## Failures' "$OUT"
    grep -q '^## Next steps' "$OUT"
    grep -qe '^- #1234 fix: bug' "$OUT"
}

@test "## Next steps section is always present with convergence fallback" {
    run bash "$HELPER" --output "$OUT" --sha 0 --branch main
    [ "$status" -eq 0 ]
    grep -q '^## Next steps' "$OUT"
    # Empty / missing next-steps file → canonical convergence sentinel.
    grep -qF -- '- (none — converged)' "$OUT"
}

@test "## Next steps content from --next-steps-file flows verbatim" {
    NEXT="$TMP_DIR/next.md"
    cat > "$NEXT" <<'EOF'
- Investigate flaky test in repo/foo
- Open follow-up issue for caching
EOF
    run bash "$HELPER" \
        --next-steps-file "$NEXT" \
        --output "$OUT" --sha 0 --branch main
    [ "$status" -eq 0 ]
    grep -qF -- '- Investigate flaky test in repo/foo' "$OUT"
    grep -qF -- '- Open follow-up issue for caching' "$OUT"
    # Convergence sentinel must NOT appear when real content is supplied.
    ! grep -qF -- '- (none — converged)' "$OUT"
}

@test "harvest from fresh run-summary extracts next-steps content" {
    NEXT="$TMP_DIR/next.md"
    printf -- '- Follow up on PR #999\n- Retry issue #500\n' > "$NEXT"
    run bash "$HELPER" \
        --next-steps-file "$NEXT" \
        --output "$OUT" --sha 0 --branch main
    [ "$status" -eq 0 ]
    # Simulate the --continue harvester: extract lines under ## Next steps.
    harvested=$(awk '/^## Next steps$/{f=1; next} f && /^## /{f=0} f' "$OUT" \
        | grep -E '^- ' || true)
    [ -n "$harvested" ]
    echo "$harvested" | grep -qF 'Follow up on PR #999'
    echo "$harvested" | grep -qF 'Retry issue #500'
}

@test "atomic write: no leftover .tmp file on success" {
    run bash "$HELPER" --output "$OUT" --sha 0 --branch main
    [ "$status" -eq 0 ]
    [ -f "$OUT" ]
    # No stray temp files in the output directory.
    ! ls "$TMP_DIR"/*.tmp.* >/dev/null 2>&1
}
