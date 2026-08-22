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

extract_shell_slice() {
    awk -v begin="$1" -v end="$2" '
        index($0, begin) { inside=1; next }
        index($0, end) { exit }
        inside { line=$0; sub(/^> ?/, "", line); print line }
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
        esac ;;
    diff) printf '%s\n' "${SIZE_CASE:-pass}" ;;
    fetch|cat-file) exit 0 ;;
    push) printf 'push\n' >> "$MUTATION_LOG" ;;
    *) exit 1 ;;
esac
EOF
cat > "$TEST_TMPDIR/bin/gh" <<'EOF'
#!/usr/bin/env bash
case "$1 $2" in
    "pr view") printf '%s\t%s\n' "${REMOTE_BASE_OID:-base-oid}" "${REMOTE_HEAD_OID:-head-oid}" ;;
    "pr merge")
        case " $* " in
            *" --admin --squash --delete-branch --match-head-commit ${REMOTE_HEAD_OID:-head-oid} "*) printf 'merge\n' >> "$MUTATION_LOG" ;;
            *) exit 9 ;;
        esac ;;
    *) exit 1 ;;
esac
EOF
    cat > "$TEST_TMPDIR/scripts/autospec-guarded-merge.sh" <<'EOF'
#!/usr/bin/env bash
pr=""; args=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --pr) pr="$2"; shift 2 ;;
        --repo) shift 2 ;;
        --merge-args) args="$2"; shift 2 ;;
        *) exit 2 ;;
    esac
done
case "${MATCH_MODE:-exact}" in
    missing) args="--admin --squash --delete-branch" ;;
    wrong) args="--admin --squash --delete-branch --match-head-commit wrong-head" ;;
esac
# shellcheck disable=SC2086
gh pr merge "$pr" $args
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
        "$TEST_TMPDIR/scripts/autospec-guarded-merge.sh" \
        "$TEST_TMPDIR/scripts/lint-implementation.sh"
}

run_pr_size_slice() {
    local boundary="$1" size_case="$2" skill_name="${3:-autospec-run}" skill script mutation
    skill="$REPO_ROOT/skills/$skill_name/SKILL.md"
    mutation="$TEST_TMPDIR/mutations"
    : > "$mutation"
    script="$(extract_shell_slice '# pr-size-helper:begin' '# pr-size-helper:end' "$skill")"
    script="${script}"$'\n'"$(extract_shell_slice \
        "# pr-size-${boundary}-exec:begin" "# pr-size-${boundary}-exec:end" "$skill")"
    if [ "$boundary" = "pre-push" ]; then
        script="${script}"$'\n''git push'
    else
        script="${script}"$'\n'"$(extract_shell_slice \
            '# pr-size-guarded-merge-exec:begin' '# pr-size-guarded-merge-exec:end' "$skill")"$'\n''run_guarded_pr_size_merge'
    fi
    script="${script//<ISSUE>/2737}"
    script="${script//<PR>/77}"
    script="${script//<BRANCH>/feat/test}"
    run env PATH="$TEST_TMPDIR/bin:$PATH" \
        AUTOSPEC_SCRIPTS_DIR="$TEST_TMPDIR/scripts" \
        MUTATION_LOG="$mutation" SIZE_CASE="$size_case" \
        REMOTE_BASE_OID="${REMOTE_BASE_OID:-base-oid}" \
        REMOTE_HEAD_OID="${REMOTE_HEAD_OID:-head-oid}" \
        LOCAL_HEAD_OID="${LOCAL_HEAD_OID:-head-oid}" \
        FETCHED_BASE_OID="${FETCHED_BASE_OID:-base-oid}" \
        FETCHED_HEAD_OID="${FETCHED_HEAD_OID:-head-oid}" \
        MATCH_MODE="${MATCH_MODE:-exact}" \
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

@test "PR_SIZE autospec and autospec-run executable blocks byte-equal" {
    for block in helper pre-push-exec final-merge-exec guarded-merge-exec; do
        diff \
            <(extract_shell_slice "# pr-size-$block:begin" "# pr-size-$block:end" "$REPO_ROOT/skills/autospec/SKILL.md") \
            <(extract_shell_slice "# pr-size-$block:begin" "# pr-size-$block:end" "$REPO_ROOT/skills/autospec-run/SKILL.md")
    done
}

@test "PR_SIZE pre-push rejects 401 lines 9 files and 4 units before mutation" {
    make_pr_size_fakes
    for skill in autospec-run autospec; do
        for size_case in 401 9 4; do
            run_pr_size_slice pre-push "$size_case" "$skill"
            [ "$status" -ne 0 ]
            ! grep -q . "$TEST_TMPDIR/mutations"
        done
    done
}

@test "PR_SIZE exact acceptance is required at push and final merge boundaries" {
    make_pr_size_fakes
    for skill in autospec-run autospec; do
    run_pr_size_slice pre-push pass "$skill"
    [ "$status" -eq 0 ]
    grep -qxF push "$TEST_TMPDIR/mutations"

    run_pr_size_slice final-merge near "$skill"
    [ "$status" -ne 0 ]
    [ ! -s "$TEST_TMPDIR/mutations" ]

    run_pr_size_slice final-merge pass "$skill"
    [ "$status" -eq 0 ] || { echo "$output"; return 1; }
    grep -qxF merge "$TEST_TMPDIR/mutations" || { echo "$output"; return 1; }

    MATCH_MODE=missing run_pr_size_slice final-merge pass "$skill"
    ! grep -q . "$TEST_TMPDIR/mutations"

    MATCH_MODE=wrong run_pr_size_slice final-merge pass "$skill"
    ! grep -q . "$TEST_TMPDIR/mutations"
    done
}

@test "PR_SIZE final merge fails closed on remote head or fetched branch drift" {
    make_pr_size_fakes
    for skill in autospec-run autospec; do
    REMOTE_HEAD_OID=remote-head LOCAL_HEAD_OID=stale-head \
        run_pr_size_slice final-merge pass "$skill"
    [ "$status" -ne 0 ]
    ! grep -q . "$TEST_TMPDIR/mutations"

    REMOTE_HEAD_OID=remote-head LOCAL_HEAD_OID=remote-head FETCHED_HEAD_OID=stale-head \
        run_pr_size_slice final-merge pass "$skill"
    [ "$status" -ne 0 ]
    ! grep -q . "$TEST_TMPDIR/mutations"
    done
}

# ── issue #3101: run_pr_size_gate must not read positional parameters ────────

# The harness substitutes its slash-command argument into $1/$2/$3 when it
# renders a skill body, so the helper defined inside that body must take its
# inputs from named variables only. These fakes differ from make_pr_size_fakes:
# `git diff` here rejects anything other than the exact expected OIDs, so a
# clobbered PR_SIZE_BASE_OID cannot look like a pass, and the linter records
# that it ran so "fails closed" can be asserted as "never reached the linter".
make_pr_size_env_fakes() {
    cat > "$TEST_TMPDIR/bin/git" <<'EOF'
#!/usr/bin/env bash
case "$1" in
    diff)
        if [ "$3" = "base-oid" ] && [ "$4" = "head-oid" ]; then
            printf 'diff-body\n'
        else
            printf 'fatal: bad revision\n' >&2
            exit 128
        fi ;;
    *) exit 1 ;;
esac
EOF
    cat > "$TEST_TMPDIR/scripts/lint-implementation.sh" <<'EOF'
#!/usr/bin/env bash
printf 'linted\n' >> "$MUTATION_LOG"
exit 0
EOF
    chmod +x "$TEST_TMPDIR/bin/git" "$TEST_TMPDIR/scripts/lint-implementation.sh"
}

# run_pr_size_helper <extra-args-string>
# Runs the helper slice alone with PR_SIZE_* supplied as named variables.
# <extra-args-string> stands in for the harness-substituted argument.
run_pr_size_helper() {
    local extra="$1" script
    script="$(extract_shell_slice '# pr-size-helper:begin' '# pr-size-helper:end' \
        "$REPO_ROOT/skills/autospec-run/SKILL.md")"
    script="${script//<ISSUE>/3101}"
    script="${script}"$'\n'"run_pr_size_gate $extra"
    : > "$TEST_TMPDIR/mutations"
    # PR_SIZE_* are inherited from this shell's exported environment, not
    # injected here: a default injected at this seam would silently re-supply
    # the very variable the unset cases below are trying to withhold.
    run env PATH="$TEST_TMPDIR/bin:$PATH" \
        AUTOSPEC_SCRIPTS_DIR="$TEST_TMPDIR/scripts" \
        MUTATION_LOG="$TEST_TMPDIR/mutations" \
        bash -c "$script"
}

@test "PR_SIZE gate ignores a harness-substituted argument and reads named vars" {
    make_pr_size_env_fakes
    export PR_SIZE_PHASE=pre-push PR_SIZE_BASE_OID=base-oid PR_SIZE_HEAD_OID=head-oid
    # A rendered body sees the caller's slash-command argument in $1. If the
    # helper still read positionals, PR_SIZE_BASE_OID/HEAD_OID would be clobbered
    # to empty here and `git diff` would reject them.
    run_pr_size_helper "'/autospec-run --profile local'"
    [ "$status" -eq 0 ] || { echo "$output"; return 1; }
    grep -qxF 'INFO:PR_SIZE: acceptance' <<< "$output"
    grep -qxF linted "$TEST_TMPDIR/mutations"
}

@test "PR_SIZE gate fails closed when any named input is unset or empty" {
    make_pr_size_env_fakes
    for missing in PR_SIZE_PHASE PR_SIZE_BASE_OID PR_SIZE_HEAD_OID; do
        for value in "" "unset"; do
            export PR_SIZE_PHASE=pre-push PR_SIZE_BASE_OID=base-oid PR_SIZE_HEAD_OID=head-oid
            if [ "$value" = unset ]; then
                unset "$missing"
            else
                export "$missing="
            fi
            run_pr_size_helper ""
            [ "$status" -ne 0 ] || { echo "$missing/$value accepted: $output"; return 1; }
            # A named diagnostic, not an incidental `git diff` blow-up: the gate
            # has to recognise its own missing input rather than trip over it.
            grep -q 'ERROR:PR_SIZE' <<< "$output" \
                || { echo "$missing/$value gave no ERROR:PR_SIZE line: $output"; return 1; }
            # Fail closed means the linter is never reached, not just a bad exit.
            [ ! -s "$TEST_TMPDIR/mutations" ] || { echo "$missing/$value reached linter"; return 1; }
        done
    done
    unset PR_SIZE_PHASE PR_SIZE_BASE_OID PR_SIZE_HEAD_OID
}
