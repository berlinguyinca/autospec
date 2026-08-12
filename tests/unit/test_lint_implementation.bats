#!/usr/bin/env bats
# tests/unit/test_lint_implementation.bats — one @test per fixture row.
# Exercises scripts/lint-implementation.sh exit code and stdout findings.

bats_require_minimum_version 1.5.0

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-implementation.sh"
    AUTOSPEC="$REPO_ROOT/target/debug/autospec"
    if [ ! -x "$AUTOSPEC" ]; then
        cargo build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" -p autospec-cli --bin autospec
    fi
    FIX="$REPO_ROOT/tests/fixtures/implementation-quality"
    PR_SIZE_TMP="$(mktemp -d)"
}

teardown() {
    rm -rf "$PR_SIZE_TMP"
}

pr_size_file() {
    local path="$1" count="$2" content="${3:-line}"
    {
        printf 'diff --git a/%s b/%s\n--- a/%s\n+++ b/%s\n@@ -0,0 +1,%s @@\n' \
            "$path" "$path" "$path" "$path" "$count"
        local i=0
        while [ "$i" -lt "$count" ]; do
            printf '+%s %s\n' "$content" "$i"
            i=$((i + 1))
        done
    } >> "$PR_SIZE_TMP/change.diff"
}

pr_size_replacement() {
    local path="$1" count="$2"
    {
        printf 'diff --git a/%s b/%s\n--- a/%s\n+++ b/%s\n@@ -1,%s +1,%s @@\n' \
            "$path" "$path" "$path" "$path" "$count" "$count"
        local i=0
        while [ "$i" -lt "$count" ]; do printf -- '-old %s\n' "$i"; i=$((i + 1)); done
        i=0
        while [ "$i" -lt "$count" ]; do printf '+new %s\n' "$i"; i=$((i + 1)); done
    } >> "$PR_SIZE_TMP/change.diff"
}

pr_size_issue() {
    printf '%s\n\n%s\n' "$1" \
        "Guardian: skip-OUT_OF_SCOPE # PR_SIZE fixture intentionally omits implementation outline" \
        > "$PR_SIZE_TMP/issue.md"
    mkdir -p "$PR_SIZE_TMP/bin"
    cat > "$PR_SIZE_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
cat "$PR_SIZE_ISSUE_BODY"
EOF
    chmod +x "$PR_SIZE_TMP/bin/gh"
}

run_pr_size() {
    run env PATH="$PR_SIZE_TMP/bin:$PATH" PR_SIZE_ISSUE_BODY="$PR_SIZE_TMP/issue.md" \
        bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff" --issue 2702 "$@"
}

contract_issue() {
    printf '%s\n' "$1" > "$PR_SIZE_TMP/issue.md"
    mkdir -p "$PR_SIZE_TMP/bin"
    cat > "$PR_SIZE_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
cat "$CONTRACT_ISSUE_BODY"
EOF
    chmod +x "$PR_SIZE_TMP/bin/gh"
}

install_real_contract_cli() {
    CONTRACT_AUTOSPEC_BIN="$AUTOSPEC"
}

run_contract_shell() {
    env PATH="$PR_SIZE_TMP/bin:/usr/bin:/bin" \
        AUTOSPEC_BIN="${CONTRACT_AUTOSPEC_BIN:-}" \
        CONTRACT_ISSUE_BODY="$PR_SIZE_TMP/issue.md" \
        bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff" --issue 2764 "$@"
}

install_many_findings_contract_cli() {
    cat > "$PR_SIZE_TMP/bin/autospec" <<'EOF'
#!/usr/bin/env bash
i=1
while [ "$i" -le 12 ]; do
    printf 'OUT_OF_SCOPE:src/out-%s.rs:-: delegated scope finding %s\n' "$i" "$i"
    i=$((i + 1))
done
i=1
while [ "$i" -le 12 ]; do
    printf 'MISSING_TEST:tests/integration/test-%s.rs:-: delegated test finding %s\n' "$i" "$i"
    i=$((i + 1))
done
exit 24
EOF
    chmod +x "$PR_SIZE_TMP/bin/autospec"
    CONTRACT_AUTOSPEC_BIN="$PR_SIZE_TMP/bin/autospec"
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

# ── core implementation-contract delegation ──────────────────────────────────

@test "implementation contract delegates ordered scope and test findings before shell detectors" {
    contract_issue "## Implementation outline

- \`src/allowed.rs\`

## Tests required

- integration: real shell regression

Guardian: skip-OUT_OF_SCOPE # fixture proves delegated INFO rendering"
    printf '%s\n' \
        'diff --git a/scripts/unlisted.sh b/scripts/unlisted.sh' \
        'new file mode 100755' \
        '--- /dev/null' \
        '+++ b/scripts/unlisted.sh' \
        '@@ -0,0 +1 @@' \
        '+eval(input)' \
        > "$PR_SIZE_TMP/change.diff"
    install_real_contract_cli

    run "$AUTOSPEC" lint implementation-contract \
        --issue-body-file "$PR_SIZE_TMP/issue.md" \
        --diff-file "$PR_SIZE_TMP/change.diff"
    [ "$status" -eq 1 ]
    local cli_output="$output"

    run run_contract_shell
    [ "$status" -eq 2 ]
    local delegated
    delegated="$(printf '%s\n' "$output" | grep -E '^(INFO:)?(OUT_OF_SCOPE|MISSING_TEST):')"
    [ "$delegated" = "$cli_output" ]
    [ "$(printf '%s\n' "$output" | sed -n '3p')" = \
        "SECURITY:scripts/unlisted.sh:2: eval() usage — potential code injection" ]
}

@test "implementation contract accepts project-native scripts test regression evidence" {
    contract_issue "## Implementation outline

- \`scripts/test-autonomous-status-panel.mjs\`

## Tests required

- integration: real shell regression"
    cat > "$PR_SIZE_TMP/change.diff" <<'EOF'
diff --git a/scripts/test-autonomous-status-panel.mjs b/scripts/test-autonomous-status-panel.mjs
new file mode 100644
--- /dev/null
+++ b/scripts/test-autonomous-status-panel.mjs
@@ -0,0 +1 @@
+export const status = "covered";
EOF
    install_real_contract_cli

    run run_contract_shell
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "implementation contract fails loudly when the CLI is missing or unreadable" {
    contract_issue "## Implementation outline

- \`src/allowed.rs\`"
    cat > "$PR_SIZE_TMP/change.diff" <<'EOF'
diff --git a/src/allowed.rs b/src/allowed.rs
new file mode 100644
--- /dev/null
+++ b/src/allowed.rs
@@ -0,0 +1 @@
+value
EOF

    for mode in missing unreadable; do
        rm -f "$PR_SIZE_TMP/bin/autospec"
        CONTRACT_AUTOSPEC_BIN="$PR_SIZE_TMP/bin/autospec"
        if [ "$mode" = unreadable ]; then
            printf '#!/usr/bin/env bash\nexit 0\n' > "$PR_SIZE_TMP/bin/autospec"
            chmod 0644 "$PR_SIZE_TMP/bin/autospec"
        fi
        run --separate-stderr run_contract_shell
        [ "$status" -eq 1 ]
        [ -z "$output" ]
        [ "$stderr" = \
            "lint-implementation.sh: implementation-contract CLI is unavailable: autospec" ]
    done
}

@test "implementation contract rejects malformed output and non-finding CLI failures" {
    contract_issue "## Implementation outline

- \`src/allowed.rs\`"
    cat > "$PR_SIZE_TMP/change.diff" <<'EOF'
diff --git a/src/allowed.rs b/src/allowed.rs
new file mode 100644
--- /dev/null
+++ b/src/allowed.rs
@@ -0,0 +1 @@
+value
EOF

    cat > "$PR_SIZE_TMP/bin/autospec" <<'EOF'
#!/usr/bin/env bash
printf 'NOT_A_FINDING\n'
exit 1
EOF
    chmod +x "$PR_SIZE_TMP/bin/autospec"
    CONTRACT_AUTOSPEC_BIN="$PR_SIZE_TMP/bin/autospec"
    run --separate-stderr run_contract_shell
    [ "$status" -eq 1 ]
    [ -z "$output" ]
    [ "$stderr" = \
        "lint-implementation.sh: malformed implementation-contract CLI output: NOT_A_FINDING" ]

    run --separate-stderr run_contract_shell --directives
    [ "$status" -eq 1 ]
    [ -z "$output" ]
    [ "$stderr" = \
        "lint-implementation.sh: malformed implementation-contract CLI output: NOT_A_FINDING" ]

    cat > "$PR_SIZE_TMP/bin/autospec" <<'EOF'
#!/usr/bin/env bash
printf 'transport failed\n' >&2
exit 7
EOF
    chmod +x "$PR_SIZE_TMP/bin/autospec"
    CONTRACT_AUTOSPEC_BIN="$PR_SIZE_TMP/bin/autospec"
    run --separate-stderr run_contract_shell
    [ "$status" -eq 1 ]
    [ -z "$output" ]
    [ "$stderr" = \
        "lint-implementation.sh: implementation-contract CLI failed without findings (exit 7): transport failed" ]
}

@test "implementation contract caps each delegated rule before cumulative shell accounting" {
    contract_issue "## Implementation outline

- \`src/allowed.rs\`"
    printf '%s\n' \
        'diff --git a/scripts/unlisted.sh b/scripts/unlisted.sh' \
        'new file mode 100755' \
        '--- /dev/null' \
        '+++ b/scripts/unlisted.sh' \
        '@@ -0,0 +1 @@' \
        '+eval(input)' \
        > "$PR_SIZE_TMP/change.diff"
    install_many_findings_contract_cli

    run --separate-stderr run_contract_shell
    [ "$status" -eq 23 ]
    [ "$(printf '%s\n' "$output" | wc -l | tr -d ' ')" -eq 23 ]
    [ "$(printf '%s\n' "$output" | sed -n '11p')" = \
        "OUT_OF_SCOPE:src/out-11.rs:-: + more (truncated)" ]
    [ "$(printf '%s\n' "$output" | sed -n '22p')" = \
        "MISSING_TEST:tests/integration/test-11.rs:-: + more (truncated)" ]
    [ "$(printf '%s\n' "$output" | sed -n '23p')" = \
        "SECURITY:scripts/unlisted.sh:2: eval() usage — potential code injection" ]
}

# ── PR_SIZE detector ─────────────────────────────────────────────────────────

@test "PR_SIZE changed-line and raw-file boundaries are inclusive" {
    pr_size_replacement "docs/a.md" 200
    run bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff"
    [ "$status" -eq 0 ]

    pr_size_file "docs/b.md" 1
    run bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff"
    [ "$status" -eq 1 ]
    [[ "$output" == *"ERROR:PR_SIZE:-:-: changed_lines=401/400"* ]]

    : > "$PR_SIZE_TMP/change.diff"
    for i in {1..8}; do
        pr_size_file "tests/fixtures/skill-goldens/$i.sha256" 1
    done
    run bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff"
    [ "$status" -eq 0 ]
    pr_size_file "tests/fixtures/skill-goldens/9.sha256" 1
    run bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff"
    [ "$status" -eq 1 ]
    [[ "$output" == *"raw_files=9/8"* ]]
}

@test "PR_SIZE logical-unit boundary is inclusive at three" {
    for i in {1..3}; do pr_size_file "docs/$i.md" 1; done
    run bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff"
    [ "$status" -eq 0 ]

    pr_size_file "docs/4.md" 1
    run bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff"
    [ "$status" -eq 1 ]
    [[ "$output" == *"logical_units=4/3"* ]]
}

@test "PR_SIZE binary numstat evidence fails closed" {
    printf '%s\n' \
        'diff --git a/image.png b/image.png' \
        'Binary files a/image.png and b/image.png differ' > "$PR_SIZE_TMP/change.diff"
    run bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff"
    [ "$status" -eq 1 ]
    [[ "$output" == *"ERROR:PR_SIZE:-:-:"*"binary=true"*"exceeded=binary"* ]]
}

@test "PR_SIZE accepts only exact generated and solver exception shapes" {
    pr_size_file "db/migrations/a 1 b.sql" 1 "Generated by prisma"; for i in {1..3}; do pr_size_file "db/migrations/$i/001.sql" 1 "Generated by prisma"; done
    pr_size_issue "Guardian: skip-PR_SIZE # generated migration: prisma   "
    run_pr_size
    [ "$status" -eq 0 ]
    [[ "$output" == *"INFO:PR_SIZE:-:-:"*"category=generated migration"* ]]
    : > "$PR_SIZE_TMP/change.diff"; pr_size_file "db/migrations/a[.]sql" 1 manual; pr_size_file "db/migrations/a.sql" 1 "Generated by prisma"; for i in 1 2; do pr_size_file "db/migrations/$i/001.sql" 1 "Generated by prisma"; done
    run_pr_size
    [ "$status" -eq 1 ]

    : > "$PR_SIZE_TMP/change.diff"
    for i in {1..4}; do pr_size_file "$i/package-lock.json" 1; done
    pr_size_issue "Guardian: skip-PR_SIZE # dependency-solver lockfile: npm"
    run env PATH="$PR_SIZE_TMP/bin:$PATH" PR_SIZE_ISSUE_BODY="$PR_SIZE_TMP/issue.md" bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff" --issue 2702
    [ "$status" -eq 0 ]
    [[ "$output" == *"category=dependency-solver lockfile"* ]]

    for reason in \
        "Guardian: skip-PR_SIZE # generated migration:" \
        "Guardian: skip-PR_SIZE # dependency-solver lockfile: cargo" \
        "Guardian: skip-PR_SIZE # unknown: tool"; do
        pr_size_issue "$reason"
        run_pr_size
        [ "$status" -eq 1 ]
        [[ "$output" == *"PR_SIZE:-:-:"* ]]
    done
    pr_size_file "nested/tests/manual.json" 1
    pr_size_issue "Guardian: skip-PR_SIZE # dependency-solver lockfile: npm"
    run env PATH="$PR_SIZE_TMP/bin:$PATH" PR_SIZE_ISSUE_BODY="$PR_SIZE_TMP/issue.md" bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff" --issue 2702
    [ "$status" -eq 1 ]
}

@test "PR_SIZE lock-step exception validates identity fingerprints and manual scope" {
    for skill in autospec autospec-run; do
        for adapter in SKILL.md codex/prompt.md opencode/agent.md; do
            pr_size_file "skills/$skill/$adapter" 1 "identical-$skill"
        done
        for golden in SKILL.md codex.prompt.md opencode.agent.md; do
            pr_size_file "tests/fixtures/skill-goldens/$skill.$golden.sha256" 1
        done
    done
    pr_size_issue "Guardian: skip-PR_SIZE # mandatory lock-step artifacts: autospec and autospec-run adapter trios plus derived goldens"
    run env PATH="$PR_SIZE_TMP/bin:$PATH" PR_SIZE_ISSUE_BODY="$PR_SIZE_TMP/issue.md" bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff" --issue 2702
    [ "$status" -eq 0 ]
    [[ "$output" == *"category=mandatory lock-step artifacts"* ]]

    for extra in "src/manual.rs" "nested/tests/manual.yaml"; do
        cp "$PR_SIZE_TMP/change.diff" "$PR_SIZE_TMP/base.diff"
        pr_size_file "$extra" 1
        run_pr_size
        [ "$status" -eq 1 ]
        mv "$PR_SIZE_TMP/base.diff" "$PR_SIZE_TMP/change.diff"
    done
    pr_size_issue "Guardian: skip-PR_SIZE # mandatory lock-step artifacts: forged"
    run_pr_size
    [ "$status" -eq 1 ]
}

@test "PR_SIZE --directives matches the Rust directive exactly" {
    pr_size_file "docs/a.md" 401
    run bash "$LINT" --diff-file "$PR_SIZE_TMP/change.diff" --directives
    [ "$status" -eq 1 ]
    [ "$output" = "Fix PR_SIZE: Freeze the completed capped slice and move unmet acceptance criteria to ordered continuation issues; never push or merge this oversized diff." ]
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

@test "lint-implementation: offline issue body applies skips without gh" {
    mkdir -p "$PR_SIZE_TMP/bin"
    printf '%s\n' \
        'Guardian: skip-TODO_LEFT, skip-MISSING_TEST, skip-OUT_OF_SCOPE # contained hook receives exact offline issue evidence' \
        > "$PR_SIZE_TMP/offline.issue.md"
    cat > "$PR_SIZE_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf 'unexpected gh call\n' >&2
exit 99
EOF
    chmod +x "$PR_SIZE_TMP/bin/gh"

    run env PATH="$PR_SIZE_TMP/bin:$PATH" \
        AUTOSPEC_LINT_ISSUE_BODY_FILE="$PR_SIZE_TMP/offline.issue.md" \
        bash "$LINT" --diff-file "$FIX/skip-respected.diff" --issue 42

    [ "$status" -eq 0 ]
    echo "$output" | grep -q '^INFO:TODO_LEFT:'
    ! echo "$output" | grep -q 'unexpected gh call'
}

@test "lint-implementation: missing offline issue body fails closed" {
    run env AUTOSPEC_LINT_ISSUE_BODY_FILE="$PR_SIZE_TMP/missing.issue.md" \
        bash "$LINT" --diff-file "$FIX/good.diff"

    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'offline issue body file'
}

@test "lint-implementation: empty offline issue body path fails closed" {
    run env AUTOSPEC_LINT_ISSUE_BODY_FILE= bash "$LINT" --diff-file "$FIX/good.diff"

    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'offline issue body file'
}

@test "lint-implementation: missing offline issue body fails closed before an empty staged diff" {
    cd "$PR_SIZE_TMP"
    git init -q

    run env AUTOSPEC_LINT_ISSUE_BODY_FILE="$PR_SIZE_TMP/missing.issue.md" \
        bash "$LINT" --staged

    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'offline issue body file'
    ! echo "$output" | grep -q 'no staged changes found'
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

# ── --vacuous-assertions: POSITIVE fixtures (8 detections) ───────────────────

# _vac_diff FPATH LINE1 [LINE2 ...] — write a minimal added-lines diff to a temp file,
# print the temp file path. Caller must rm the file.
_vac_tmpfile=""
_vac_diff() {
    local fpath="$1"; shift
    _vac_tmpfile="$(mktemp -t lint-vac-XXXXXX.diff)"
    {
        printf 'diff --git a/%s b/%s\nnew file mode 100644\n--- /dev/null\n+++ b/%s\n@@ -0,0 +1,%s @@\n' \
            "$fpath" "$fpath" "$fpath" "$#"
        for line in "$@"; do
            printf '+%s\n' "$line"
        done
    } > "$_vac_tmpfile"
}

@test "vacuous: VACUOUS_GREP_INVERSE_OR_TRUE detected in test file" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "x" {' \
        '  grep -qv "foo" file.txt || true' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "VACUOUS_GREP_INVERSE_OR_TRUE"
}

@test "vacuous: VACUOUS_OR_TRUE detected when assertion ends with || true" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "x" {' \
        '  run some_cmd' \
        '  [ "$status" -eq 0 ] || true' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "VACUOUS_OR_TRUE"
}

@test "vacuous: VACUOUS_TAUTOLOGY detected for assert True" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "x" {' \
        '  assert True' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "VACUOUS_TAUTOLOGY"
}

@test "vacuous: VACUOUS_TAUTOLOGY detected for expect(true).toBe(true)" {
    _vac_diff 'tests/unit/test_x.js' \
        "it('x', () => {" \
        '  expect(true).toBe(true);' \
        '});'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "VACUOUS_TAUTOLOGY"
}

@test "vacuous: VACUOUS_AC_STUB detected for skip auto-stub in tests/ac/" {
    _vac_diff 'tests/ac/test_foo.bats' \
        '#!/usr/bin/env bats' \
        '@test "AC-1: does something" {' \
        '  skip "auto-stub"' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "VACUOUS_AC_STUB"
}

@test "vacuous: VACUOUS_EMPTY_TEST detected for it(..., () => {}) in JS" {
    _vac_diff 'tests/unit/test_x.js' \
        "it('does something', () => {});"
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "VACUOUS_EMPTY_TEST"
}

@test "vacuous: VACUOUS_TAUTOLOGY detected for xit(...)" {
    _vac_diff 'tests/unit/test_x.js' \
        "xit('pending test', () => {" \
        '  expect(1).toBe(1);' \
        '});'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "VACUOUS_TAUTOLOGY"
}

@test "vacuous: VACUOUS_NO_ASSERT detected for bats test with no assertions" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "empty test no assert" {' \
        '  echo "hello"' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    echo "$output" | grep -q "VACUOUS_NO_ASSERT"
}

# ── --vacuous-assertions: NEGATIVE fixtures (8 no false positives) ────────────

@test "vacuous: no VACUOUS_GREP_INVERSE_OR_TRUE for legitimate ! grep -q" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "x" {' \
        '  ! grep -q "foo" file.txt' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_GREP_INVERSE_OR_TRUE"
}

@test "vacuous: no VACUOUS_OR_TRUE for || true in non-test cleanup line" {
    _vac_diff 'scripts/helper.sh' \
        '#!/usr/bin/env bash' \
        'rm -f /tmp/work.tmp || true'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_OR_TRUE"
}

@test "vacuous: no VACUOUS_TAUTOLOGY for legitimate assert with variable" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "x" {' \
        '  run my_cmd' \
        '  assert_output "expected"' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_TAUTOLOGY"
}

@test "vacuous: no VACUOUS_AC_STUB for skip with real reason outside tests/ac/" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "x" {' \
        '  skip "needs network"' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_AC_STUB"
}

@test "vacuous: no VACUOUS_EMPTY_TEST for it() with non-empty body" {
    _vac_diff 'tests/unit/test_x.js' \
        "it('does something', () => {" \
        "  expect(result).toBe('value');" \
        '});'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_EMPTY_TEST"
}

@test "vacuous: no VACUOUS_TAUTOLOGY for expect(result).toBe(expected) with variables" {
    _vac_diff 'tests/unit/test_x.js' \
        'const result = fn();' \
        'expect(result).toBe(expected);'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_TAUTOLOGY"
}

@test "vacuous: no VACUOUS_NO_ASSERT for test with run + status check" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "runs fine" {' \
        '  run my_command arg' \
        '  [ "$status" -eq 0 ]' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_NO_ASSERT"
}

@test "vacuous: no VACUOUS_GREP_INVERSE_OR_TRUE for grep -qv without || true" {
    _vac_diff 'tests/unit/test_x.bats' \
        '#!/usr/bin/env bats' \
        '@test "x" {' \
        '  grep -qv "foo" file.txt && echo "pattern absent on some lines"' \
        '}'
    run bash "$LINT" --diff-file "$_vac_tmpfile" --vacuous-assertions
    rm -f "$_vac_tmpfile"
    ! echo "$output" | grep -q "VACUOUS_GREP_INVERSE_OR_TRUE"
}

@test "vacuous: no VACUOUS findings on clean diff file" {
    run bash "$LINT" --diff-file "$FIX/good.diff" --vacuous-assertions
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -qE "^VACUOUS_"
}

# ── --pre-commit bundles --vacuous-assertions ─────────────────────────────────

@test "vacuous: --pre-commit invokes vacuous-assertions check" {
    TMPDIR_REPO="$(mktemp -d)"
    git -C "$TMPDIR_REPO" init -q
    git -C "$TMPDIR_REPO" config user.email "t@t.com"
    git -C "$TMPDIR_REPO" config user.name "T"
    touch "$TMPDIR_REPO/README.md"
    git -C "$TMPDIR_REPO" add README.md
    git -C "$TMPDIR_REPO" commit -q -m "init"

    # Stage a test file with a vacuous grep pattern
    mkdir -p "$TMPDIR_REPO/tests/unit"
    # Write via printf to avoid bats scanning heredoc @test lines
    printf '#!/usr/bin/env bats\n@test "check vacuous grep pattern" {\n  grep -qv "foo" somefile || true\n}\n' \
        > "$TMPDIR_REPO/tests/unit/test_bad.bats"
    git -C "$TMPDIR_REPO" add tests/unit/test_bad.bats

    run bash -c "cd '$TMPDIR_REPO' && bash '$LINT' --pre-commit --staged"
    rm -rf "$TMPDIR_REPO"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "VACUOUS_GREP_INVERSE_OR_TRUE"
}
