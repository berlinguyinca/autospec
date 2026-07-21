#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    INSTALL_SCRIPT="$REPO_ROOT/scripts/install-implementer-precommit.sh"
    LINT_SCRIPT="$REPO_ROOT/scripts/lint-implementation.sh"
    TEST_ROOT="$(mktemp -d)"
    SOURCE_REPO="$TEST_ROOT/source"
    WORKTREE="$TEST_ROOT/worktree"

    git init -q "$SOURCE_REPO"
    git -C "$SOURCE_REPO" config user.email "test@example.com"
    git -C "$SOURCE_REPO" config user.name "Test User"
    touch "$SOURCE_REPO/README.md"
    git -C "$SOURCE_REPO" add README.md
    git -C "$SOURCE_REPO" commit -q -m "initial"
    git -C "$SOURCE_REPO" worktree add -q -b fix/2371-hook "$WORKTREE"
}

teardown() {
    rm -rf "$TEST_ROOT"
}

make_recording_linter() {
    mkdir -p "$TEST_ROOT/scripts"
    cat > "$TEST_ROOT/scripts/lint-implementation.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" > "$CAPTURE_FILE"
EOF
    chmod +x "$TEST_ROOT/scripts/lint-implementation.sh"
}

make_issue_gh() {
    mkdir -p "$TEST_ROOT/bin"
    cat > "$TEST_ROOT/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' 'Guardian: skip-TODO_LEFT # covered by issue 2371 regression fixture'
EOF
    chmod +x "$TEST_ROOT/bin/gh"
}

write_todo_diff() {
    cat > "$TEST_ROOT/todo.diff" <<'EOF'
diff --git a/scripts/example.sh b/scripts/example.sh
new file mode 100755
--- /dev/null
+++ b/scripts/example.sh
@@ -0,0 +1,2 @@
+#!/usr/bin/env bash
+# TODO: exercise the documented issue exemption
EOF
}

@test "installer writes an executable hook to Git's resolved linked-worktree path" {
    run bash "$INSTALL_SCRIPT" "$WORKTREE"
    [ "$status" -eq 0 ]

    hook_path="$(git -C "$WORKTREE" rev-parse --git-path hooks/pre-commit)"
    [ -x "$hook_path" ]
}

@test "installed hook passes the numeric branch issue to staged lint" {
    make_recording_linter
    export AUTOSPEC_SCRIPTS_DIR="$TEST_ROOT/scripts"
    export CAPTURE_FILE="$TEST_ROOT/args"
    bash "$INSTALL_SCRIPT" "$WORKTREE"
    printf '%s\n' changed > "$WORKTREE/change.txt"
    git -C "$WORKTREE" add change.txt

    run git -C "$WORKTREE" commit -m "test numeric issue"
    [ "$status" -eq 0 ]
    [ -f "$CAPTURE_FILE" ]
    grep -q -- '--pre-commit --staged --issue 2371' "$CAPTURE_FILE"
}

@test "installed hook omits issue arguments on a branch without a numeric issue segment" {
    make_recording_linter
    export AUTOSPEC_SCRIPTS_DIR="$TEST_ROOT/scripts"
    export CAPTURE_FILE="$TEST_ROOT/args"
    git -C "$WORKTREE" checkout -q -b fix/hook-without-issue
    bash "$INSTALL_SCRIPT" "$WORKTREE"
    printf '%s\n' changed > "$WORKTREE/change.txt"
    git -C "$WORKTREE" add change.txt

    run git -C "$WORKTREE" commit -m "test missing issue"
    [ "$status" -eq 0 ]
    [ -f "$CAPTURE_FILE" ]
    [ "$(cat "$CAPTURE_FILE")" = "--pre-commit --staged" ]
}

@test "diff-file mode loads issue skip directives before detectors run" {
    make_issue_gh
    write_todo_diff

    run env PATH="$TEST_ROOT/bin:$PATH" bash "$LINT_SCRIPT" \
        --diff-file "$TEST_ROOT/todo.diff" --issue 2371
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'INFO:TODO_LEFT:'
}

@test "staged mode loads issue skip directives before detectors run" {
    make_issue_gh
    cat > "$WORKTREE/example.sh" <<'EOF'
#!/usr/bin/env bash
# TODO: exercise the documented issue exemption
EOF
    git -C "$WORKTREE" add example.sh

    run bash -c "cd '$WORKTREE' && PATH='$TEST_ROOT/bin:$PATH' bash '$LINT_SCRIPT' --staged --issue 2371"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'INFO:TODO_LEFT:'
}
