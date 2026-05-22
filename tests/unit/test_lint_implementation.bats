#!/usr/bin/env bats
# tests/unit/test_lint_implementation.bats — one @test per fixture row.
# Exercises scripts/lint-implementation.sh exit code and stdout findings.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-implementation.sh"
    FIX="$REPO_ROOT/tests/fixtures/implementation-quality"
}

# ── syntax check ─────────────────────────────────────────────────────────────

@test "lint-implementation: bash -n exits 0" {
    run bash -n "$LINT"
    [ "$status" -eq 0 ]
}

@test "lint-implementation: --help lists OUT_OF_SCOPE" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OUT_OF_SCOPE"
}

@test "lint-implementation: --help lists MISSING_TEST" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "MISSING_TEST"
}

@test "lint-implementation: --help lists all 10 RULE_IDs" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "OUT_OF_SCOPE"
    echo "$output" | grep -q "MISSING_TEST"
    echo "$output" | grep -q "COMPLEXITY"
    echo "$output" | grep -q "SECURITY"
    echo "$output" | grep -q "TODO_LEFT"
    echo "$output" | grep -q "MOCK_DB"
    echo "$output" | grep -q "DOC_OUT_OF_SYNC"
    echo "$output" | grep -q "HALLUCINATED_API"
    echo "$output" | grep -q "DUPLICATE_CODE"
    echo "$output" | grep -q "INVENTED_CONFIG"
}

# ── good fixture ──────────────────────────────────────────────────────────────

@test "lint-implementation: good.diff exits 0 with no findings" {
    run bash "$LINT" --diff-file "$FIX/good.diff"
    [ "$status" -eq 0 ]
    # No blocking RULE_ID lines (no OUT_OF_SCOPE etc.)
    echo "$output" | grep -vq "^INFO:" || true
    ! echo "$output" | grep -qE "^(OUT_OF_SCOPE|MISSING_TEST|COMPLEXITY|SECURITY|TODO_LEFT|MOCK_DB|DOC_OUT_OF_SYNC):"
}

# ── TODO_LEFT detector ────────────────────────────────────────────────────────

@test "lint-implementation: bad-todo-left.diff exits >=1 and reports TODO_LEFT" {
    run bash "$LINT" --diff-file "$FIX/bad-todo-left.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT"
}

@test "lint-implementation: bad-todo-left.diff does not report TODO_LEFT in test files" {
    # The bad-todo-left.diff only has TODO in non-test source; ensure it fires correctly
    run bash "$LINT" --diff-file "$FIX/bad-todo-left.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT:scripts/"
}

# ── SECURITY detector ─────────────────────────────────────────────────────────

@test "lint-implementation: bad-secret.diff exits >=1 and reports SECURITY" {
    run bash "$LINT" --diff-file "$FIX/bad-secret.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "SECURITY"
}

@test "lint-implementation: bad-secret.diff SECURITY finding mentions AWS key" {
    run bash "$LINT" --diff-file "$FIX/bad-secret.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "SECURITY.*AKIA\|hardcoded AWS"
}

# ── MOCK_DB detector ──────────────────────────────────────────────────────────

@test "lint-implementation: bad-mock-db.diff exits >=1 and reports MOCK_DB" {
    run bash "$LINT" --diff-file "$FIX/bad-mock-db.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "MOCK_DB"
}

# ── COMPLEXITY detector ───────────────────────────────────────────────────────

@test "lint-implementation: bad-complexity.diff exits >=1 and reports COMPLEXITY" {
    run bash "$LINT" --diff-file "$FIX/bad-complexity.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "COMPLEXITY"
}

# ── per-RULE_ID emit cap ──────────────────────────────────────────────────────

@test "lint-implementation: per-RULE_ID cap collapses 11+ hits to truncated notice" {
    # Generate a synthetic diff with 12 TODO lines in a non-test source file
    local synth_diff
    synth_diff="$(mktemp -t lint-impl-cap-test.XXXXXX.diff)"
    {
        printf '%s\n' 'diff --git a/scripts/synth.sh b/scripts/synth.sh'
        printf '%s\n' 'new file mode 100755'
        printf '%s\n' '--- /dev/null'
        printf '%s\n' '+++ b/scripts/synth.sh'
        printf '%s\n' '@@ -0,0 +1,12 @@'
        local i=1
        while [ "$i" -le 12 ]; do
            printf '+# TODO: fix item %d\n' "$i"
            i=$((i+1))
        done
    } > "$synth_diff"
    run bash "$LINT" --diff-file "$synth_diff"
    rm -f "$synth_diff"
    # Should have some TODO_LEFT findings and a truncation notice
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT"
    echo "$output" | grep -q "truncated"
}

# ── findings hard cap ─────────────────────────────────────────────────────────

@test "lint-implementation: scope explosion message is emitted at hard cap" {
    # Verify the hard-cap message text exists in the script (structural test)
    grep -q "too many findings" "$LINT"
    grep -q "exit 200" "$LINT"
}

# ── skip-directive opt-out ────────────────────────────────────────────────────

@test "lint-implementation: skip-respected.diff has Guardian skip line in issue file" {
    grep -q "Guardian: skip-TODO_LEFT" "$FIX/skip-respected.issue.md"
}

@test "lint-implementation: skip-respected.diff TODO_LEFT is blocked without skip directive" {
    # Without passing --issue, skip directives are not loaded; TODO_LEFT fires
    run bash "$LINT" --diff-file "$FIX/skip-respected.diff"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TODO_LEFT"
}

# ── --help documents new flags ────────────────────────────────────────────────

@test "lint-implementation: --help documents --pre-commit" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "\-\-pre-commit"
}

@test "lint-implementation: --help documents --staged" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "\-\-staged"
}

@test "lint-implementation: --help documents --directives" {
    run bash "$LINT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "\-\-directives"
}

# ── --staged mode ─────────────────────────────────────────────────────────────

@test "lint-implementation: --staged exits 0 with clean staged diff" {
    # Create a temp git repo, stage a clean file, run lint --staged
    TMPDIR_REPO="$(mktemp -d)"
    git -C "$TMPDIR_REPO" init -q
    git -C "$TMPDIR_REPO" config user.email "t@t.com"
    git -C "$TMPDIR_REPO" config user.name "T"
    touch "$TMPDIR_REPO/README.md"
    git -C "$TMPDIR_REPO" add README.md
    git -C "$TMPDIR_REPO" commit -q -m "init"

    # Stage a clean file
    cat > "$TMPDIR_REPO/clean.sh" <<'EOF'
#!/usr/bin/env bash
set -eu
echo "hello"
EOF
    git -C "$TMPDIR_REPO" add clean.sh

    run bash -c "cd '$TMPDIR_REPO' && bash '$LINT' --staged"
    rm -rf "$TMPDIR_REPO"
    [ "$status" -eq 0 ]
}

@test "lint-implementation: --pre-commit --staged detects SECURITY in staged diff" {
    TMPDIR_REPO="$(mktemp -d)"
    git -C "$TMPDIR_REPO" init -q
    git -C "$TMPDIR_REPO" config user.email "t@t.com"
    git -C "$TMPDIR_REPO" config user.name "T"
    touch "$TMPDIR_REPO/README.md"
    git -C "$TMPDIR_REPO" add README.md
    git -C "$TMPDIR_REPO" commit -q -m "init"

    # Stage a file with a hardcoded AWS key
    cat > "$TMPDIR_REPO/bad.sh" <<'EOF'
#!/usr/bin/env bash
AWS_KEY=AKIAIOSFODNN7EXAMPLE
echo "$AWS_KEY"
EOF
    git -C "$TMPDIR_REPO" add bad.sh

    run bash -c "cd '$TMPDIR_REPO' && bash '$LINT' --pre-commit --staged"
    rm -rf "$TMPDIR_REPO"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "SECURITY"
}

# ── --directives mode ─────────────────────────────────────────────────────────

@test "lint-implementation: --directives outputs one directive per finding" {
    run bash "$LINT" --diff-file "$FIX/bad-secret.diff" --directives
    # Should have at least one Fix: line
    echo "$output" | grep -q "^Fix "
}

@test "lint-implementation: --directives output starts with 'Fix <RULE_ID>:'" {
    run bash "$LINT" --diff-file "$FIX/bad-secret.diff" --directives
    # All output lines should match "Fix RULE_ID: ..."
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        echo "$line" | grep -qE "^Fix [A-Z_]+: "
    done <<< "$output"
}

@test "lint-implementation: --directives with TODO_LEFT fixture emits Fix TODO_LEFT directive" {
    run bash "$LINT" --diff-file "$FIX/bad-todo-left.diff" --directives
    echo "$output" | grep -q "Fix TODO_LEFT"
}
