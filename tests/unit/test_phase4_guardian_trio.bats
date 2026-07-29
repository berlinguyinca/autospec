#!/usr/bin/env bats
# tests/unit/test_phase4_guardian_trio.bats — byte-identity assertions for the
# guardian dispatch block across all 6 Phase 4 trio files (3 per skill × 2 skills).
# Per spec §5.2 lock-step trio and §8 testing strategy.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    TEST_TMPDIR="$(mktemp -d)"
    mkdir -p "$TEST_TMPDIR/bin" "$TEST_TMPDIR/scripts"
}

teardown() {
    rm -rf "$TEST_TMPDIR"
}

# extract_guardian_block <file>
# Emits lines between the outermost <!-- guardian-block:begin --> and
# <!-- guardian-block:end --> markers, tracking nesting so that the inner
# marker pair inside the GCMT heredoc does not prematurely close the block.
extract_guardian_block() {
    awk '
        /<!-- guardian-block:begin -->/ { depth++; if (depth == 1) { next } }
        /<!-- guardian-block:end -->/   { if (depth == 1) { depth=0; next } depth-- }
        depth >= 1 { print }
    ' "$1"
}

line_of() {
    grep -nF "$1" "$2" | head -n 1 | cut -d: -f1
}

extract_shell_slice() {
    awk -v begin="$1" -v end="$2" '
        index($0, begin) { inside=1; next }
        index($0, end) { exit }
        inside {
            line=$0
            sub(/^> ?/, "", line)
            print line
        }
    ' "$3"
}

make_pr_size_fakes() {
    cat > "$TEST_TMPDIR/bin/git" <<'EOF'
#!/usr/bin/env bash
case "$1" in
    merge-base) printf '%s\n' "${LOCAL_BASE_OID:-base-oid}" ;;
    rev-parse)
        case "$2" in
            HEAD) printf '%s\n' "${LOCAL_HEAD_OID:-head-oid}" ;;
            origin/main) printf '%s\n' "${FETCHED_BASE_OID:-base-oid}" ;;
            origin/feat/test) printf '%s\n' "${FETCHED_HEAD_OID:-head-oid}" ;;
            *) exit 1 ;;
        esac
        ;;
    diff) printf '%s\n' "${SIZE_CASE:-pass}" ;;
    fetch|cat-file) exit 0 ;;
    push) printf 'push\n' >> "$MUTATION_LOG" ;;
    *) exit 1 ;;
esac
EOF
    cat > "$TEST_TMPDIR/bin/gh" <<'EOF'
#!/usr/bin/env bash
if [ "$1 $2" = "pr view" ]; then
    printf '%s\t%s\n' "${REMOTE_BASE_OID:-base-oid}" "${REMOTE_HEAD_OID:-head-oid}"
    exit 0
fi
if [ "$1 $2" = "pr merge" ]; then
    printf 'merge\n' >> "$MUTATION_LOG"
    exit 0
fi
exit 1
EOF
    cat > "$TEST_TMPDIR/scripts/lint-implementation.sh" <<'EOF'
#!/usr/bin/env bash
case "${SIZE_CASE:-pass}" in
    401) printf 'ERROR:PR_SIZE:-:-: changed_lines=401/400\n'; exit 1 ;;
    9) printf 'ERROR:PR_SIZE:-:-: raw_files=9/8\n'; exit 1 ;;
    4) printf 'ERROR:PR_SIZE:-:-: logical_units=4/3\n'; exit 1 ;;
    near) printf 'INFO:PR_SIZE: acceptance extra\n'; exit 1 ;;
    pass) exit 0 ;;
esac
EOF
    chmod +x "$TEST_TMPDIR/bin/git" "$TEST_TMPDIR/bin/gh" \
        "$TEST_TMPDIR/scripts/lint-implementation.sh"
}

run_pr_size_slice() {
    local boundary="$1" size_case="$2" skill script mutation
    skill="$REPO_ROOT/skills/autospec-run/SKILL.md"
    mutation="$TEST_TMPDIR/mutations"
    : > "$mutation"
    script="$(extract_shell_slice '# pr-size-helper:begin' '# pr-size-helper:end' "$skill")"
    script="${script}"$'\n'"$(extract_shell_slice \
        "# pr-size-${boundary}-exec:begin" "# pr-size-${boundary}-exec:end" "$skill")"
    script="${script//<ISSUE>/2737}"
    script="${script//<PR>/77}"
    script="${script//<BRANCH>/feat/test}"
    if [ "$boundary" = "pre-push" ]; then
        script="${script}"$'\n''git push'
    else
        script="${script}"$'\n''gh pr merge 77'
    fi
    run env PATH="$TEST_TMPDIR/bin:$PATH" \
        AUTOSPEC_SCRIPTS_DIR="$TEST_TMPDIR/scripts" \
        MUTATION_LOG="$mutation" SIZE_CASE="$size_case" \
        REMOTE_BASE_OID="${REMOTE_BASE_OID:-base-oid}" \
        REMOTE_HEAD_OID="${REMOTE_HEAD_OID:-head-oid}" \
        LOCAL_HEAD_OID="${LOCAL_HEAD_OID:-head-oid}" \
        FETCHED_BASE_OID="${FETCHED_BASE_OID:-base-oid}" \
        FETCHED_HEAD_OID="${FETCHED_HEAD_OID:-head-oid}" \
        bash -c "$script"
}

# ── skills/autospec ────────────────────────────────────────────────────────────

@test "autospec SKILL.md guardian block has required markers" {
    block="$(extract_guardian_block "$REPO_ROOT/skills/autospec/SKILL.md")"
    [ -n "$block" ] || { echo "guardian block empty in autospec/SKILL.md"; return 1; }
    printf '%s\n' "$block" | grep -q 'AUTOSPEC_NO_GUARDIAN' \
        || { echo "missing AUTOSPEC_NO_GUARDIAN in autospec/SKILL.md guardian block"; return 1; }
    printf '%s\n' "$block" | grep -qE 'GUARDIAN_PASS|LGTM' \
        || { echo "missing GUARDIAN_PASS or LGTM verdict in autospec/SKILL.md guardian block"; return 1; }
}

@test "autospec codex prompt guardian block byte-equals SKILL.md" {
    skill="$REPO_ROOT/skills/autospec"
    diff \
        <(extract_guardian_block "$skill/SKILL.md") \
        <(extract_guardian_block "$skill/codex/prompt.md") \
        || { echo "GUARDIAN_PASS: autospec codex/prompt.md guardian block diverges from SKILL.md"; return 1; }
}

@test "autospec opencode agent guardian block byte-equals SKILL.md" {
    skill="$REPO_ROOT/skills/autospec"
    diff \
        <(extract_guardian_block "$skill/SKILL.md") \
        <(extract_guardian_block "$skill/opencode/agent.md") \
        || { echo "GUARDIAN_PASS: autospec opencode/agent.md guardian block diverges from SKILL.md"; return 1; }
}

# ── skills/autospec-run ────────────────────────────────────────────────────────

@test "autospec-run SKILL.md guardian block has required markers" {
    block="$(extract_guardian_block "$REPO_ROOT/skills/autospec-run/SKILL.md")"
    [ -n "$block" ] || { echo "guardian block empty in autospec-run/SKILL.md"; return 1; }
    printf '%s\n' "$block" | grep -q 'AUTOSPEC_NO_GUARDIAN' \
        || { echo "missing AUTOSPEC_NO_GUARDIAN in autospec-run/SKILL.md guardian block"; return 1; }
    printf '%s\n' "$block" | grep -qE 'GUARDIAN_PASS|LGTM' \
        || { echo "missing GUARDIAN_PASS or LGTM verdict in autospec-run/SKILL.md guardian block"; return 1; }
}

@test "autospec-run codex prompt guardian block byte-equals SKILL.md" {
    skill="$REPO_ROOT/skills/autospec-run"
    diff \
        <(extract_guardian_block "$skill/SKILL.md") \
        <(extract_guardian_block "$skill/codex/prompt.md") \
        || { echo "GUARDIAN_PASS: autospec-run codex/prompt.md guardian block diverges from SKILL.md"; return 1; }
}

@test "autospec-run opencode agent guardian block byte-equals SKILL.md" {
    skill="$REPO_ROOT/skills/autospec-run"
    diff \
        <(extract_guardian_block "$skill/SKILL.md") \
        <(extract_guardian_block "$skill/opencode/agent.md") \
        || { echo "GUARDIAN_PASS: autospec-run opencode/agent.md guardian block diverges from SKILL.md"; return 1; }
}

# ── cross-skill byte-identity ──────────────────────────────────────────────────

@test "autospec vs autospec-run guardian blocks byte-equal" {
    diff \
        <(extract_guardian_block "$REPO_ROOT/skills/autospec/SKILL.md") \
        <(extract_guardian_block "$REPO_ROOT/skills/autospec-run/SKILL.md") \
        || { echo "GUARDIAN_PASS: autospec and autospec-run SKILL.md guardian blocks diverge"; return 1; }
}

@test "PR_SIZE autospec-run gates push and final merge with exact acceptance evidence" {
    for file in \
        "$REPO_ROOT/skills/autospec-run/SKILL.md" \
        "$REPO_ROOT/skills/autospec-run/codex/prompt.md" \
        "$REPO_ROOT/skills/autospec-run/opencode/agent.md"; do
        grep -qF '<!-- pr-size-pre-push:begin -->' "$file" \
            || { echo "missing PR_SIZE pre-push gate in $file"; return 1; }
        grep -qF '<!-- pr-size-final-merge:begin -->' "$file" \
            || { echo "missing PR_SIZE final-merge gate in $file"; return 1; }
        grep -qF '401 changed lines' "$file" \
            || { echo "missing 401-line rejection in $file"; return 1; }
        grep -qF '9 raw files' "$file" \
            || { echo "missing 9-file rejection in $file"; return 1; }
        grep -qF '4 logical units' "$file" \
            || { echo "missing 4-unit rejection in $file"; return 1; }
        grep -qF 'git diff --binary "$PR_SIZE_BASE_OID" "$PR_SIZE_HEAD_OID"' "$file" \
            || { echo "PR_SIZE does not lint the exact base-to-head diff in $file"; return 1; }
        grep -qF 'gh pr view <PR> --json baseRefOid,headRefOid' "$file" \
            || { echo "PR_SIZE final gate does not query live PR OIDs in $file"; return 1; }
        grep -qF "grep -qxF 'INFO:PR_SIZE: acceptance'" "$file" \
            || { echo "PR_SIZE acceptance is not exact-line matched in $file"; return 1; }

        pre_push="$(line_of '<!-- pr-size-pre-push:begin -->' "$file")"
        push="$(line_of '> 5. Push:' "$file")"
        final_merge="$(line_of '<!-- pr-size-final-merge:begin -->' "$file")"
        guarded_merge="$(line_of 'if bash "${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/autospec-guarded-merge.sh"' "$file")"
        [ "$pre_push" -lt "$push" ] \
            || { echo "PR_SIZE pre-push gate follows push in $file"; return 1; }
        [ "$final_merge" -lt "$guarded_merge" ] \
            || { echo "PR_SIZE final gate follows guarded merge in $file"; return 1; }
    done
}

@test "PR_SIZE pre-push rejects 401 lines 9 files and 4 units before mutation" {
    make_pr_size_fakes
    for size_case in 401 9 4; do
        run_pr_size_slice pre-push "$size_case"
        [ "$status" -ne 0 ]
        ! grep -q . "$TEST_TMPDIR/mutations"
    done
}

@test "PR_SIZE exact acceptance is required at push and final merge boundaries" {
    make_pr_size_fakes
    run_pr_size_slice pre-push pass
    [ "$status" -eq 0 ]
    grep -qxF push "$TEST_TMPDIR/mutations"

    run_pr_size_slice final-merge near
    [ "$status" -ne 0 ]
    [ ! -s "$TEST_TMPDIR/mutations" ]

    run_pr_size_slice final-merge pass
    [ "$status" -eq 0 ]
    grep -qxF merge "$TEST_TMPDIR/mutations"
}

@test "PR_SIZE final merge fails closed on remote head or fetched branch drift" {
    make_pr_size_fakes
    REMOTE_HEAD_OID=remote-head LOCAL_HEAD_OID=stale-head \
        run_pr_size_slice final-merge pass
    [ "$status" -ne 0 ]
    ! grep -q . "$TEST_TMPDIR/mutations"

    REMOTE_HEAD_OID=remote-head LOCAL_HEAD_OID=remote-head FETCHED_HEAD_OID=stale-head \
        run_pr_size_slice final-merge pass
    [ "$status" -ne 0 ]
    ! grep -q . "$TEST_TMPDIR/mutations"
}
