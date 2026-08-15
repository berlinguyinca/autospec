#!/usr/bin/env bats
# tests/stack-guard.bats — tests for scripts/stack-guard.sh (per-layer size + linearity gate).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    GUARD="$REPO_ROOT/scripts/stack-guard.sh"
    WORK="$(mktemp -d -t stack-guard-test.XXXXXX)"
    REPO="$WORK/repo"
    BIN="$WORK/bin"
    mkdir -p "$BIN"
    git init -q -b main "$REPO"
    git -C "$REPO" config user.email t@t
    git -C "$REPO" config user.name t
    ( cd "$REPO" && echo seed > README.md && git add . && git commit -qm seed )
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

# commit_on <branch> <file> <content> — create <branch> off the current branch, add <file>, commit.
commit_on() {
    local branch="$1" file="$2" content="$3"
    ( cd "$REPO" && git checkout -qb "$branch" && printf '%s\n' "$content" > "$file" && git add . && git commit -qm "$branch" )
}

# over_cap_file — in $REPO, write a file with 401 lines (over the 400-line cap) and commit it.
over_cap_file() {
    ( cd "$REPO" && for i in $(seq 1 401); do echo "line $i padding padding padding padding"; done > big.txt && git add . && git commit -qm overcap )
}

# stub_gh <branch> — fake gh on $BIN that reports <branch> as an open PR head.
stub_gh() {
    local out="$1"
    {
        printf '#!/usr/bin/env bash\n'
        printf 'if [ "$1" = "pr" ] && [ "$2" = "list" ]; then\n'
        printf '  printf "%%s\\n" %s\n' "'$out'"
        printf '  exit 0\n'
        printf 'fi\n'
        printf 'exit 0\n'
    } > "$BIN/gh"
    chmod +x "$BIN/gh"
}

# no_gh — fake gh on $BIN that reports no open PRs.
no_gh() { printf '#!/usr/bin/env bash\nexit 0\n' > "$BIN/gh"; chmod +x "$BIN/gh"; }

# run_guard <guard-args...> — run the guard in $REPO with the stubbed PATH, advisory mode.
run_guard() { ( cd "$REPO" && PATH="$BIN:$PATH" bash "$GUARD" --default-branch main "$@" ); }

# run_guard_strict <guard-args...> — same, but with AUTOSPEC_PR_SIZE_STRICT=1.
run_guard_strict() { ( cd "$REPO" && PATH="$BIN:$PATH" AUTOSPEC_PR_SIZE_STRICT=1 bash "$GUARD" --default-branch main "$@" ); }

# ── Syntax / invocation ───────────────────────────────────────────────────────

@test "stack-guard.sh: bash -n syntax check" {
    run bash -n "$GUARD"
    [ "$status" -eq 0 ]
}

@test "stack-guard.sh: --help exits 0" {
    run bash "$GUARD" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"Usage: scripts/stack-guard.sh"* ]]
}

@test "stack-guard.sh: missing base/head exits 2" {
    no_gh
    run run_guard
    [ "$status" -eq 2 ]
    [[ "$output" == *"need --pr N or both --base and --head"* ]]
}

# ── Per-layer size ────────────────────────────────────────────────────────────

@test "small layer under cap is OK (advisory)" {
    no_gh
    commit_on small note.txt "one"
    run run_guard --base main --head small
    [ "$status" -eq 0 ]
    [[ "$output" == *"per-layer (base...head) under cap"* ]]
    [[ "$output" == *"linear: base main == default branch main"* ]]
}

@test "over-cap layer is advisory by default (INFO, exit 0)" {
    no_gh
    commit_on big big.txt x
    over_cap_file
    run run_guard --base main --head big
    [ "$status" -eq 0 ]
    [[ "$output" == *"INFO:PR_SIZE"* ]]
    [[ "$output" == *"changed_lines="* ]]
    [[ "$output" == *"advisory=1"* ]]
}

@test "over-cap layer is blocking under strict (ERROR, exit 1)" {
    no_gh
    commit_on big big.txt x
    over_cap_file
    run run_guard_strict --base main --head big
    [ "$status" -eq 1 ]
    [[ "$output" == *"ERROR:PR_SIZE"* ]]
    [[ "$output" == *"stack-guard: FAIL"* ]]
}

# ── Linearity ─────────────────────────────────────────────────────────────────

@test "orphan base with no open PRs is non-linear (advisory INFO, exit 0)" {
    no_gh
    commit_on orphan o.txt "x"
    commit_on child c.txt "y"
    run run_guard --base orphan --head child
    [ "$status" -eq 0 ]
    [[ "$output" == *"base orphan is not main and not an open PR head"* ]]
    [[ "$output" == *"(advisory)"* ]]
}

@test "orphan base with no open PRs is blocking under strict (ERROR, exit 1)" {
    no_gh
    commit_on orphan o.txt "x"
    commit_on child c.txt "y"
    run run_guard_strict --base orphan --head child
    [ "$status" -eq 1 ]
    [[ "$output" == *"ERROR:STACK_BASE"* ]]
    [[ "$output" == *"stack-guard: FAIL"* ]]
}

@test "base that is an open PR head is linear" {
    commit_on orphan o.txt "x"
    commit_on child c.txt "y"
    stub_gh "orphan"
    run run_guard --base orphan --head child
    [ "$status" -eq 0 ]
    [[ "$output" == *"base orphan is the head of an open PR"* ]]
}

@test "--assume-linear skips the open-PR check for a non-default base" {
    no_gh
    commit_on orphan o.txt "x"
    commit_on child c.txt "y"
    run run_guard --base orphan --head child --assume-linear
    [ "$status" -eq 0 ]
    [[ "$output" == *"linear"* ]]
}
