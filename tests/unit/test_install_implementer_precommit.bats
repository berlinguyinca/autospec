#!/usr/bin/env bats
# tests/unit/test_install_implementer_precommit.bats
# Exercises scripts/install-implementer-precommit.sh

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    INSTALL_SCRIPT="$REPO_ROOT/scripts/install-implementer-precommit.sh"
    LINT_SCRIPT="$REPO_ROOT/scripts/lint-implementation.sh"

    # Create a temp git repo for each test
    TMPDIR_REPO="$(mktemp -d)"
    git -C "$TMPDIR_REPO" init -q
    git -C "$TMPDIR_REPO" config user.email "test@test.com"
    git -C "$TMPDIR_REPO" config user.name "Test"
    # Initial commit so HEAD exists
    touch "$TMPDIR_REPO/README.md"
    git -C "$TMPDIR_REPO" add README.md
    git -C "$TMPDIR_REPO" commit -q -m "init"
}

teardown() {
    rm -rf "$TMPDIR_REPO"
}

# ── syntax check ─────────────────────────────────────────────────────────────

@test "install-implementer-precommit: bash -n exits 0" {
    run bash -n "$INSTALL_SCRIPT"
    [ "$status" -eq 0 ]
}

# ── install tests ─────────────────────────────────────────────────────────────

@test "install-implementer-precommit: installs hook into .git/hooks/pre-commit" {
    run bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    [ "$status" -eq 0 ]
    [ -f "$TMPDIR_REPO/.git/hooks/pre-commit" ]
}

@test "install-implementer-precommit: installed hook is executable" {
    run bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    [ "$status" -eq 0 ]
    [ -x "$TMPDIR_REPO/.git/hooks/pre-commit" ]
}

@test "install-implementer-precommit: hook contains lint-implementation.sh invocation" {
    run bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    [ "$status" -eq 0 ]
    grep -q "lint-implementation.sh" "$TMPDIR_REPO/.git/hooks/pre-commit"
}

@test "install-implementer-precommit: hook uses --pre-commit --staged flags" {
    run bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    [ "$status" -eq 0 ]
    grep -q "\-\-pre-commit \-\-staged" "$TMPDIR_REPO/.git/hooks/pre-commit"
}

@test "lint-implementation: staged base excludes changes in the incoming merge parent" {
    git -C "$TMPDIR_REPO" branch feature
    cat > "$TMPDIR_REPO/incoming.sh" <<'EOF'
#!/usr/bin/env bash
# TODO incoming main only
echo incoming
EOF
    git -C "$TMPDIR_REPO" add incoming.sh
    git -C "$TMPDIR_REPO" commit -q -m "incoming main"
    incoming="$(git -C "$TMPDIR_REPO" rev-parse HEAD)"

    git -C "$TMPDIR_REPO" checkout -q feature
    printf '#!/usr/bin/env bash\necho feature\n' > "$TMPDIR_REPO/feature.sh"
    git -C "$TMPDIR_REPO" add feature.sh
    git -C "$TMPDIR_REPO" commit -q -m "feature"
    git -C "$TMPDIR_REPO" merge -q --no-commit --no-ff "$incoming"

    run bash -c "cd '$TMPDIR_REPO' && bash '$LINT_SCRIPT' --pre-commit --staged"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT"

    run bash -c "cd '$TMPDIR_REPO' && bash '$LINT_SCRIPT' --pre-commit --staged --staged-base '$incoming'"
    [ "$status" -eq 0 ]
}

@test "lint-implementation: staged base rejects an invalid commit" {
    printf 'staged\n' > "$TMPDIR_REPO/staged.txt"
    git -C "$TMPDIR_REPO" add staged.txt

    run bash -c "cd '$TMPDIR_REPO' && bash '$LINT_SCRIPT' --staged --staged-base not-a-commit"
    [ "$status" -eq 1 ]
    echo "$output" | grep -q "invalid staged base"
}

@test "install-implementer-precommit: ordinary staged hook omits staged base" {
    bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    mkdir -p "$TMPDIR_REPO/lint-bin"
    cat > "$TMPDIR_REPO/lint-bin/lint-implementation.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$LINT_ARGS"
EOF
    chmod +x "$TMPDIR_REPO/lint-bin/lint-implementation.sh"
    printf 'ordinary\n' > "$TMPDIR_REPO/ordinary.txt"
    git -C "$TMPDIR_REPO" add ordinary.txt

    run bash -c "cd '$TMPDIR_REPO' && AUTOSPEC_SCRIPTS_DIR='$TMPDIR_REPO/lint-bin' \
        LINT_ARGS='$TMPDIR_REPO/lint.args' .git/hooks/pre-commit"
    [ "$status" -eq 0 ]
    [ -f "$TMPDIR_REPO/lint.args" ]
    ! grep -q -- "--staged-base" "$TMPDIR_REPO/lint.args"
}

@test "install-implementer-precommit: merge hook passes the valid incoming parent" {
    bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    mkdir -p "$TMPDIR_REPO/lint-bin"
    cat > "$TMPDIR_REPO/lint-bin/lint-implementation.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$LINT_ARGS"
EOF
    chmod +x "$TMPDIR_REPO/lint-bin/lint-implementation.sh"
    incoming="$(git -C "$TMPDIR_REPO" rev-parse HEAD)"
    printf 'merge\n' > "$TMPDIR_REPO/merge.txt"
    git -C "$TMPDIR_REPO" add merge.txt
    printf '%s\n' "$incoming" > "$TMPDIR_REPO/.git/MERGE_HEAD"

    run bash -c "cd '$TMPDIR_REPO' && AUTOSPEC_SCRIPTS_DIR='$TMPDIR_REPO/lint-bin' \
        LINT_ARGS='$TMPDIR_REPO/lint.args' .git/hooks/pre-commit"
    [ "$status" -eq 0 ]
    grep -A1 -x -- "--staged-base" "$TMPDIR_REPO/lint.args" | grep -q "$incoming"
}

@test "install-implementer-precommit: invalid merge head fails closed" {
    bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    printf 'not-a-commit\n' > "$TMPDIR_REPO/.git/MERGE_HEAD"

    run bash -c "cd '$TMPDIR_REPO' && .git/hooks/pre-commit"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q "MERGE_HEAD"
}

@test "install-implementer-precommit: prints success message" {
    run bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "pre-commit"
}

# ── error cases ───────────────────────────────────────────────────────────────

@test "install-implementer-precommit: exits 1 with no arguments" {
    run bash "$INSTALL_SCRIPT"
    [ "$status" -eq 1 ]
}

@test "install-implementer-precommit: exits 1 when path has no .git" {
    TMPDIR_PLAIN="$(mktemp -d)"
    run bash "$INSTALL_SCRIPT" "$TMPDIR_PLAIN"
    [ "$status" -eq 1 ]
    rm -rf "$TMPDIR_PLAIN"
}

@test "install-implementer-precommit: exits 1 when path does not exist" {
    run bash "$INSTALL_SCRIPT" "/nonexistent/path"
    [ "$status" -eq 1 ]
}

# ── hook behavior: allows clean commit ───────────────────────────────────────

@test "install-implementer-precommit: hook allows commit with no staged changes" {
    bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"
    # Point hook's lint to our real lint script via AUTOSPEC_SCRIPTS_DIR
    export AUTOSPEC_SCRIPTS_DIR="$REPO_ROOT/scripts"
    # Attempt commit with nothing staged — hook should exit 0 (no staged = skip)
    run git -C "$TMPDIR_REPO" commit --allow-empty -m "empty commit"
    [ "$status" -eq 0 ]
}

# ── hook behavior: blocks RULE_ID violation ───────────────────────────────────

@test "install-implementer-precommit: hook blocks commit with SECURITY violation" {
    bash "$INSTALL_SCRIPT" "$TMPDIR_REPO"

    # Write hook to use our repo's lint script
    cat > "$TMPDIR_REPO/.git/hooks/pre-commit" <<HOOK
#!/usr/bin/env bash
set -euo pipefail
STAGED=\$(git diff --cached --name-only)
[ -z "\$STAGED" ] && exit 0

OUT=\$(mktemp -t autospec-precommit.XXXXXX)
trap 'rm -f "\$OUT"' EXIT

if ! bash "${REPO_ROOT}/scripts/lint-implementation.sh" --pre-commit --staged > "\$OUT" 2>&1; then
  echo "Pre-commit lint FAILED. Findings:" >&2
  cat "\$OUT" >&2
  exit 1
fi
HOOK
    chmod 755 "$TMPDIR_REPO/.git/hooks/pre-commit"

    # Stage a file with a hardcoded AWS key (SECURITY violation)
    cat > "$TMPDIR_REPO/bad.sh" <<'EOF'
#!/bin/bash
KEY=AKIAIOSFODNN7EXAMPLE
echo "$KEY"
EOF
    git -C "$TMPDIR_REPO" add bad.sh

    run git -C "$TMPDIR_REPO" commit -m "bad commit"
    [ "$status" -ne 0 ]
}
