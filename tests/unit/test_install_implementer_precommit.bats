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
