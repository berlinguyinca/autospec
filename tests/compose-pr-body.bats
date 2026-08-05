#!/usr/bin/env bats
# tests/compose-pr-body.bats — deterministic PR-body assembly.
#
# The point of the script is that everything except the summary comes from the
# branch, so the tests assert on facts git holds rather than on wording. The
# summary is the one part a model contributes and must survive verbatim.

COMPOSE="${BATS_TEST_DIRNAME}/../scripts/compose-pr-body.sh"

setup() {
    TMP="$(mktemp -d "${BATS_TMPDIR:-/tmp}/compose-pr-body-XXXXXX")"
    REPO="$TMP/repo"
    mkdir -p "$REPO"
    cd "$REPO" || return 1
    git init -q -b main .
    git config user.email t@example.com
    git config user.name Tester
    printf 'base\n' > base.txt
    git add base.txt
    git commit -q -m "chore: base"
    git branch -f start HEAD
}

teardown() {
    cd /
    rm -rf "$TMP"
}

# commit_change <file> <subject>
commit_change() {
    printf '%s\n' "$2" > "$1"
    git add "$1"
    git commit -q -m "$2"
}

@test "compose-pr-body.sh is executable" {
    run test -x "$COMPOSE"
    [ "$status" -eq 0 ]
}

@test "--issue is required and must be numeric" {
    run bash "$COMPOSE" --base start
    [ "$status" -eq 1 ]
    [[ "$output" == *"--issue is required"* ]]
    run bash "$COMPOSE" --issue not-a-number --base start
    [ "$status" -eq 1 ]
    [[ "$output" == *"positive integer"* ]]
}

@test "the body opens with the closing reference" {
    commit_change a.txt "feat(x): add a"
    run bash "$COMPOSE" --issue 42 --base start
    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | head -1)" = "Closes #42" ]
}

@test "the change list is the commit subjects, oldest first" {
    commit_change a.txt "feat(x): add a"
    commit_change b.txt "test(x): cover a"
    run bash "$COMPOSE" --issue 42 --base start
    [ "$status" -eq 0 ]
    # Only the Changes section; the Verification line is also a bullet.
    changes="$(printf '%s\n' "$output" | sed -n '/^## Changes$/,/^## Verification$/p' | grep -- '^- ')"
    [ "$(printf '%s\n' "$changes" | grep -c .)" -eq 2 ]
    [ "$(printf '%s\n' "$changes" | head -1)" = "- feat(x): add a" ]
    [ "$(printf '%s\n' "$changes" | tail -1)" = "- test(x): cover a" ]
}

@test "an empty commit range exits 3 so no PR is opened" {
    # A body that merely looked sparse would let an empty PR through.
    run bash "$COMPOSE" --issue 42 --base HEAD --head HEAD
    [ "$status" -eq 3 ]
    [[ "$output" == *"nothing to open a PR for"* ]]
}

# ── the summary is the model's contribution and must not be rewritten ─────────

@test "the summary file is emitted verbatim, markdown and all" {
    commit_change a.txt "feat(x): add a"
    printf 'Rejected the **obvious** approach because it\n- loses the `why`\n' > "$TMP/sum.md"
    run bash "$COMPOSE" --issue 42 --base start --summary-file "$TMP/sum.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"Rejected the **obvious** approach because it"* ]]
    [[ "$output" == *"loses the \`why\`"* ]]
}

@test "a missing summary file is an error, not a silently thinner body" {
    commit_change a.txt "feat(x): add a"
    run bash "$COMPOSE" --issue 42 --base start --summary-file "$TMP/absent.md"
    [ "$status" -eq 1 ]
    [[ "$output" == *"--summary-file not found"* ]]
}

@test "omitting the summary still yields a valid body" {
    commit_change a.txt "feat(x): add a"
    run bash "$COMPOSE" --issue 42 --base start
    [ "$status" -eq 0 ]
    [[ "$output" == *"Closes #42"* ]]
    [[ "$output" == *"## Changes"* ]]
}

# ── verification line ─────────────────────────────────────────────────────────

@test "the acceptance-suite test count is read from the file, not run" {
    commit_change a.txt "feat(x): add a"
    mkdir -p tests/ac
    printf '@test "one" {\n  run true\n}\n@test "two" {\n  run true\n}\n' > tests/ac/issue-42.bats
    run bash "$COMPOSE" --issue 42 --base start
    [ "$status" -eq 0 ]
    [[ "$output" == *"tests/ac/issue-42.bats"* ]]
    [[ "$output" == *"2 acceptance test(s)"* ]]
}

@test "an absent acceptance suite is stated rather than omitted" {
    # A missing verification line reads as "no tests were needed".
    commit_change a.txt "feat(x): add a"
    run bash "$COMPOSE" --issue 42 --base start
    [ "$status" -eq 0 ]
    [[ "$output" == *"no acceptance-criteria suite at"* ]]
}

@test "--ac-test overrides the derived suite path" {
    commit_change a.txt "feat(x): add a"
    printf '@test "only" {\n  run true\n}\n' > "$TMP/custom.bats"
    run bash "$COMPOSE" --issue 42 --base start --ac-test "$TMP/custom.bats"
    [ "$status" -eq 0 ]
    [[ "$output" == *"1 acceptance test(s)"* ]]
}

# ── determinism ───────────────────────────────────────────────────────────────

@test "the same branch yields byte-identical bodies" {
    commit_change a.txt "feat(x): add a"
    commit_change b.txt "test(x): cover a"
    bash "$COMPOSE" --issue 42 --base start > "$TMP/one.md"
    bash "$COMPOSE" --issue 42 --base start > "$TMP/two.md"
    run cmp -s "$TMP/one.md" "$TMP/two.md"
    [ "$status" -eq 0 ]
}

@test "the body carries the Claude Code trailer" {
    commit_change a.txt "feat(x): add a"
    run bash "$COMPOSE" --issue 42 --base start
    [ "$status" -eq 0 ]
    [[ "$output" == *"Generated with [Claude Code]"* ]]
}
