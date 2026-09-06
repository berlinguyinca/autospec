#!/usr/bin/env bats
# tests/unit/test_verify_produced_work.bats — guards #3535.
#
# The produced-work check asked one question: "is `git status --porcelain`
# empty?" An empty tree was read as "the task did nothing". Two ways that lie:
#
#   * A subagent that committed its work leaves a clean tree. The gate failed
#     the run that actually finished and passed the one that left a stray
#     scratch file.
#   * A `git` that was not installed, or a base ref that did not resolve, made
#     the count command print nothing. Nothing was read as zero commits ahead,
#     and "zero" was read as "no work produced" — a verdict about a repository
#     that was never inspected.
#
# The rule this pins: produced work is `uncommitted changes OR commits ahead`,
# each side measured on its own, and a side that could not be measured is
# `unknown` — which is neither 0 nor a verdict of "no work".
#
# Every case builds a REAL git repository with REAL commits so the counts come
# from git, and uses a genuinely empty PATH for the missing-tool case.

ROOT="${BATS_TEST_DIRNAME}/../.."
CHECK="$ROOT/scripts/verify-produced-work.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    REPO="$TEST_TMP/repo"
    NOBIN="$TEST_TMP/nobin"
    mkdir -p "$REPO" "$NOBIN"
    export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@e GIT_COMMITTER_NAME=t \
        GIT_COMMITTER_EMAIL=t@e GIT_CONFIG_GLOBAL=/dev/null
    git init -q -b main "$REPO"
    echo seed >"$REPO/seed.txt"
    git -C "$REPO" add seed.txt
    git -C "$REPO" commit -q -m seed
    git -C "$REPO" branch base
    # Worktree state is clean and zero commits ahead of `base`.
    unset AUTOSPEC_BASE_REF
}

teardown() { rm -rf "$TEST_TMP"; }

check() { sh "$CHECK" --repo-root "$REPO" --base base "$@"; }

# --- the headline bug: committed work is work -------------------------------

@test "a committed change on a clean tree still counts as produced work" {
    echo feature >"$REPO/feature.rs"
    git -C "$REPO" add feature.rs
    git -C "$REPO" commit -q -m "feat: add the feature"

    # `git status --porcelain` is empty here. The old check called that "no work".
    [ -z "$(git -C "$REPO" status --porcelain)" ]

    run check
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "produced-work: yes"
    echo "$output" | grep -q "commits_ahead=1"
    echo "$output" | grep -q "uncommitted_changes=0"
}

@test "an uncommitted change counts as produced work" {
    echo scratch >>"$REPO/seed.txt"
    run check
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "produced-work: yes"
    echo "$output" | grep -q "uncommitted_changes=1"
}

@test "an untracked file counts as produced work" {
    echo new >"$REPO/untracked.rs"
    run check
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "produced-work: yes"
}

@test "both sides are counted, not just one" {
    echo feature >"$REPO/feature.rs"
    git -C "$REPO" add feature.rs
    git -C "$REPO" commit -q -m "feat: one"
    echo more >>"$REPO/seed.txt"
    run check
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "uncommitted_changes=1"
    echo "$output" | grep -q "commits_ahead=1"
}

# --- zero is a measurement, and it is available ------------------------------

@test "a clean tree at base is measured as no work, and says so numerically" {
    run check
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "produced-work: no"
    # Both sides carry real numbers. `no` is only reachable when both were
    # measured, which is what separates it from `unknown`.
    echo "$output" | grep -q "uncommitted_changes=0"
    echo "$output" | grep -q "commits_ahead=0"
}

# --- the toolchain is asserted before anything is measured -------------------

@test "git absent is unavailable, names git, and measures nothing" {
    # Empty PATH: every git command would print nothing, and nothing counted is
    # zero changes and zero commits ahead — the verdict "no work produced" for a
    # repository that was never opened.
    run env PATH="$NOBIN" /bin/sh "$CHECK" --repo-root "$REPO" --base base
    [ "$status" -eq 3 ]
    echo "$output" | grep -q "UNAVAILABLE"
    echo "$output" | grep -q "git"
    if echo "$output" | grep -q "produced-work: no"; then
        fail "a tool-less run concluded that no work was produced: $output"
    fi
}

@test "the unavailable record carries unknown counts, not zeros" {
    run env PATH="$NOBIN" /bin/sh "$CHECK" --repo-root "$REPO" --base base --json
    [ "$status" -eq 3 ]
    echo "$output" | grep -q '"status":"UNAVAILABLE"'
    echo "$output" | grep -q '"uncommitted_changes":"unknown"'
    echo "$output" | grep -q '"commits_ahead":"unknown"'
    if echo "$output" | grep -q '"commits_ahead":0'; then
        fail "unmeasured commits were recorded as the number 0"
    fi
}

@test "a directory that is not a repository is unknown, not zero work" {
    run sh "$CHECK" --repo-root "$TEST_TMP" --base base
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "UNKNOWN"
    if echo "$output" | grep -q "produced-work: no"; then
        fail "a non-repository was measured as producing no work: $output"
    fi
}

# --- an unresolvable base is unknown, not zero commits ahead -----------------

@test "an unresolvable base is unknown rather than zero commits ahead" {
    run sh "$CHECK" --repo-root "$REPO" --base refs/heads/no-such-branch
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "produced-work: unknown"
    echo "$output" | grep -q "commits_ahead=unknown"
}

@test "a dirty tree is still work when the base cannot be resolved" {
    # The verdict is monotone in what was measured: one positive side is enough
    # for `yes`, so an unmeasured side cannot erase a dirty tree. `no` remains
    # the only conclusion that requires both sides to have been measured.
    echo scratch >>"$REPO/seed.txt"
    run sh "$CHECK" --repo-root "$REPO" --base refs/heads/no-such-branch
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "produced-work: yes"
    echo "$output" | grep -q "commits_ahead=unknown"
}

@test "no base and no upstream is unknown, never a silent zero" {
    run env -u AUTOSPEC_BASE_REF sh "$CHECK" --repo-root "$REPO"
    [ "$status" -eq 2 ]
    echo "$output" | grep -q "base=unknown"
}

@test "AUTOSPEC_BASE_REF supplies the base without a flag" {
    echo feature >"$REPO/feature.rs"
    git -C "$REPO" add feature.rs
    git -C "$REPO" commit -q -m "feat: via env base"
    run env AUTOSPEC_BASE_REF=base sh "$CHECK" --repo-root "$REPO"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "commits_ahead=1"
}

# --- the status record distinguishes unknown from 0 for every field ---------

@test "the json record separates 0 from unknown across both measurements" {
    run check --json
    [ "$status" -eq 1 ]
    echo "$output" | grep -q '"status":"NONE"'
    echo "$output" | grep -q '"uncommitted_changes":0'
    echo "$output" | grep -q '"commits_ahead":0'

    echo scratch >>"$REPO/seed.txt"
    run check --base refs/heads/no-such-branch --json
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"status":"WORK"'
    echo "$output" | grep -q '"uncommitted_changes":1'
    echo "$output" | grep -q '"commits_ahead":"unknown"'
    echo "$output" | grep -q '"base":"unknown"'
}

@test "usage errors exit 64 rather than reporting no work" {
    run sh "$CHECK" --definitely-not-a-flag
    [ "$status" -eq 64 ]
    run sh "$CHECK" --repo-root "$TEST_TMP/does-not-exist"
    [ "$status" -eq 64 ]
}

# --- portability -------------------------------------------------------------

@test "the check is POSIX sh and parses under dash" {
    head -1 "$CHECK" | grep -q '^#!/usr/bin/env sh$'
    if command -v dash >/dev/null 2>&1; then
        run dash -n "$CHECK"
        [ "$status" -eq 0 ]
    fi
    if grep -nE '\[\[|\bPIPESTATUS\b|\+=\(|declare -|<\(' "$CHECK"; then
        fail "verify-produced-work.sh uses a bash-only construct"
    fi
}
