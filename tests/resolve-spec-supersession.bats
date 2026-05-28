#!/usr/bin/env bats
# tests/resolve-spec-supersession.bats — coverage for scripts/resolve-spec-supersession.sh
#
# Cases (issue #635 acceptance):
#   - no overlap                  → exit 1, no winner
#   - single spec mentions key    → that spec wins
#   - newer of two overlapping    → newest commit wins
#   - three-way overlap           → most recent wins
#   - deleted spec excluded       → deleted file not considered even if it referenced the key
#
# All tests run inside a fresh tmpdir with a synthetic docs/specs/ layout. We
# force commit timestamps via GIT_AUTHOR_DATE / GIT_COMMITTER_DATE so the
# recency ordering is deterministic.

SCRIPT="${BATS_TEST_DIRNAME}/../scripts/resolve-spec-supersession.sh"

setup() {
    TMPDIR_T="$(mktemp -d)"
    cd "$TMPDIR_T"
    mkdir -p docs/specs
    git init -q
    git config user.email "test@example.com"
    git config user.name "Test"
    git commit -q --allow-empty -m "init"
}

teardown() {
    cd /
    rm -rf "$TMPDIR_T"
}

commit_spec() {
    local path="$1"
    local body="$2"
    local epoch="$3"
    printf '%s\n' "$body" > "$path"
    git add "$path"
    GIT_AUTHOR_DATE="@${epoch} +0000" GIT_COMMITTER_DATE="@${epoch} +0000" \
        git commit -q -m "add ${path}"
}

@test "exit 1 when no spec covers the behavior key" {
    commit_spec docs/specs/2026-01-01-foo.md "# Foo spec
button is blue" 1700000000

    run bash "$SCRIPT" "nonexistent-behavior-key-zzz"
    [ "$status" -eq 1 ]
    [ -z "$output" ]
}

@test "single overlapping spec wins outright" {
    commit_spec docs/specs/2026-01-01-foo.md "# Foo spec
the login button is blue" 1700000000

    run bash "$SCRIPT" "login button"
    [ "$status" -eq 0 ]
    [ "$output" = "docs/specs/2026-01-01-foo.md" ]
}

@test "newer of two overlapping specs wins (recency)" {
    commit_spec docs/specs/2026-01-01-old.md "# Old spec
the login button is blue" 1700000000
    commit_spec docs/specs/2026-02-01-new.md "# New spec
the login button is red" 1700500000

    run bash "$SCRIPT" "login button"
    [ "$status" -eq 0 ]
    [ "$output" = "docs/specs/2026-02-01-new.md" ]
}

@test "three-way overlap returns most recent" {
    commit_spec docs/specs/2026-01-01-a.md "# A
shopping cart shows totals" 1700000000
    commit_spec docs/specs/2026-02-01-b.md "# B
shopping cart shows totals with tax" 1700500000
    commit_spec docs/specs/2026-03-01-c.md "# C
shopping cart shows totals with tax and shipping" 1701000000

    run bash "$SCRIPT" "shopping cart"
    [ "$status" -eq 0 ]
    [ "$output" = "docs/specs/2026-03-01-c.md" ]
}

@test "deleted spec is excluded from candidates" {
    commit_spec docs/specs/2026-01-01-keep.md "# Keep
checkout flow uses 2FA" 1700000000
    commit_spec docs/specs/2026-02-01-delete.md "# Delete
checkout flow is single-step" 1700500000
    # Now remove the newer spec on disk (and from git).
    git rm -q docs/specs/2026-02-01-delete.md
    GIT_AUTHOR_DATE="@1701000000 +0000" GIT_COMMITTER_DATE="@1701000000 +0000" \
        git commit -q -m "remove delete spec"

    run bash "$SCRIPT" "checkout flow"
    [ "$status" -eq 0 ]
    [ "$output" = "docs/specs/2026-01-01-keep.md" ]
}

@test "--list-overlapping prints all candidates oldest-first" {
    commit_spec docs/specs/2026-01-01-old.md "feature alpha" 1700000000
    commit_spec docs/specs/2026-02-01-mid.md "feature alpha" 1700500000
    commit_spec docs/specs/2026-03-01-new.md "feature alpha" 1701000000

    run bash "$SCRIPT" --list-overlapping "feature alpha"
    [ "$status" -eq 0 ]
    [ "$(printf '%s\n' "$output" | head -1)" = "docs/specs/2026-01-01-old.md" ]
    [ "$(printf '%s\n' "$output" | tail -1)" = "docs/specs/2026-03-01-new.md" ]
}

@test "--json emits winner + ranked candidates" {
    commit_spec docs/specs/2026-01-01-old.md "feature beta" 1700000000
    commit_spec docs/specs/2026-02-01-new.md "feature beta" 1700500000

    run bash "$SCRIPT" --json "feature beta"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"winner":"docs/specs/2026-02-01-new.md"'
    echo "$output" | grep -q '"behavior":"feature beta"'
}

@test "--specs-dir override searches alternate directory" {
    mkdir -p other/specs
    commit_spec other/specs/2026-01-01-foo.md "# Foo
alternate-tree-key here" 1700000000

    run bash "$SCRIPT" --specs-dir other/specs "alternate-tree-key"
    [ "$status" -eq 0 ]
    [ "$output" = "other/specs/2026-01-01-foo.md" ]
}

@test "case-insensitive substring match" {
    commit_spec docs/specs/2026-01-01-foo.md "# Foo
The Submit Button is green" 1700000000

    run bash "$SCRIPT" "submit button"
    [ "$status" -eq 0 ]
    [ "$output" = "docs/specs/2026-01-01-foo.md" ]
}

@test "--help prints usage and exits 0" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '^Usage:'
}

@test "missing behavior key exits 2" {
    run bash "$SCRIPT"
    [ "$status" -eq 2 ]
}
