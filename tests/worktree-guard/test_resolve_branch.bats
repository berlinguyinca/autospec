#!/usr/bin/env bats
# tests/worktree-guard/test_resolve_branch.bats — `resolve-branch` JSON ladder.
#
# Covers (docs/specs/2026-06-03-worktree-guard-design.md §D1 G2 ladder, issue
# #959 Shared contracts): the deterministic verdict ladder, exit 0 always.
#   open-pr     — `gh pr list --head B --state open` non-empty -> {"state":"open-pr","pr":N}
#   branch-only — else `git ls-remote --heads origin B` non-empty -> {"state":"branch-only","pr":null}
#   fresh       — else -> {"state":"fresh","pr":null}
#
# `gh` and `git ls-remote` are PATH-shadowed mocks driven by env vars, mirroring
# the repo's established pattern (tests/resume/*.bats). The mock git falls
# through to real git for everything except `ls-remote`.

ROOT="${BATS_TEST_DIRNAME}/../.."
GUARD="$ROOT/scripts/worktree-guard.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export MOCK_DIR="$TEST_TMP/bin"
    mkdir -p "$MOCK_DIR"
    export PATH="$MOCK_DIR:$PATH"

    # Mock drivers.
    export GH_PR_JSON="[]"        # what `gh pr list ... --json number` returns
    export LSREMOTE_OUT=""        # non-empty => branch exists on origin

    write_gh_mock
    write_git_mock
}

teardown() { rm -rf "$TEST_TMP"; }

write_gh_mock() {
    cat > "$MOCK_DIR/gh" <<'EOF'
#!/usr/bin/env bash
# PATH-shadow gh mock. Supports `gh pr list --head B --state open --json number`.
if [ "$1 $2" = "pr list" ]; then
    printf '%s\n' "$GH_PR_JSON"
    exit 0
fi
exit 0
EOF
    chmod +x "$MOCK_DIR/gh"
}

write_git_mock() {
    REAL_GIT="$(command -v git)"
    export REAL_GIT
    cat > "$MOCK_DIR/git" <<EOF
#!/usr/bin/env bash
REAL_GIT="$REAL_GIT"
EOF
    cat >> "$MOCK_DIR/git" <<'EOF'
# Intercept `git ls-remote --heads origin <B>`; everything else -> real git.
for a in "$@"; do
    if [ "$a" = "ls-remote" ]; then
        printf '%s' "$LSREMOTE_OUT"
        [ -n "$LSREMOTE_OUT" ] && printf '\n'
        exit 0
    fi
done
exec "$REAL_GIT" "$@"
EOF
    chmod +x "$MOCK_DIR/git"
}

@test "resolve-branch: open PR -> open-pr verdict with pr number, exit 0" {
    export GH_PR_JSON='[{"number":42}]'
    run bash "$GUARD" resolve-branch --branch feat/x --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *'"state":"open-pr"'* ]]
    [[ "$output" == *'"pr":42'* ]]
}

@test "resolve-branch: no PR but branch on remote -> branch-only, exit 0" {
    export GH_PR_JSON='[]'
    export LSREMOTE_OUT="deadbeef refs/heads/feat/x"
    run bash "$GUARD" resolve-branch --branch feat/x --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *'"state":"branch-only"'* ]]
    [[ "$output" == *'"pr":null'* ]]
}

@test "resolve-branch: no PR and no remote branch -> fresh, exit 0" {
    export GH_PR_JSON='[]'
    export LSREMOTE_OUT=""
    run bash "$GUARD" resolve-branch --branch feat/x --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *'"state":"fresh"'* ]]
    [[ "$output" == *'"pr":null'* ]]
}

@test "resolve-branch: open-pr takes precedence over an existing remote branch" {
    export GH_PR_JSON='[{"number":7}]'
    export LSREMOTE_OUT="deadbeef refs/heads/feat/x"
    run bash "$GUARD" resolve-branch --branch feat/x --repo o/n
    [ "$status" -eq 0 ]
    [[ "$output" == *'"state":"open-pr"'* ]]
    [[ "$output" == *'"pr":7'* ]]
}

@test "resolve-branch: emits valid JSON parseable by jq" {
    export GH_PR_JSON='[{"number":99}]'
    run bash -c "bash '$GUARD' resolve-branch --branch feat/x --repo o/n | jq -r .state"
    [ "$status" -eq 0 ]
    [ "$output" = "open-pr" ]
}

@test "resolve-branch: missing --branch is a usage error (exit 2)" {
    run bash "$GUARD" resolve-branch --repo o/n
    [ "$status" -eq 2 ]
}

@test "resolve-branch: missing --repo is a usage error (exit 2)" {
    run bash "$GUARD" resolve-branch --branch feat/x
    [ "$status" -eq 2 ]
}
