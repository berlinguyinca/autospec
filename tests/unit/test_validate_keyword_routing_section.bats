#!/usr/bin/env bats

VALIDATE_SH="${BATS_TEST_DIRNAME}/../../scripts/validate.sh"

setup() {
    TMP_REPO="$(mktemp -d)"
    mkdir -p "$TMP_REPO/skills/autospec-listen/opencode" \
        "$TMP_REPO/skills/autospec-listen/codex" \
        "$TMP_REPO/skills/autospec-shared/tests/unit" \
        "$TMP_REPO/scripts"

    cat >"$TMP_REPO/scripts/listener-match.sh" <<'SCRIPT'
#!/usr/bin/env bash
# classifier literals mirrored by scripts/validate.sh keyword-routing checks
# explore-confirm
# autospec fix imperative
# post_approval_execution_ready
# plan_exit_ready
SCRIPT

    {
        printf '%s\n' '#!/usr/bin/env bats'
        printf '%s\n' '@''test "post-approval: open auto-implement issues route to autospec-run" { true; }'
        printf '%s\n' '@''test "plan-exit: completed saved implementation plan routes to autospec autonomous" { true; }'
        printf '%s\n' '@''test "plan-exit: destructive action gate does not route" { true; }'
    } >"$TMP_REPO/skills/autospec-shared/tests/unit/listener-match.bats"
}

teardown() {
    rm -rf "$TMP_REPO"
}

write_keyword_trio_file() {
    local path="$1"
    cat >"$path" <<'MARKDOWN'
# autospec-listen fixture

## Keyword auto-routing

| trigger | route | gate |
| `explore` | /autospec-explore | explore-confirm |
| `fix` | /autospec | none |

- post_approval_execution_ready
- AUTOSPEC_LISTENER_AUTO_IMPLEMENT_OPEN
- plan_exit_ready
- AUTOSPEC_LISTENER_PLAN_EXIT_READY
MARKDOWN
}

write_complete_keyword_trio() {
    write_keyword_trio_file "$TMP_REPO/skills/autospec-listen/SKILL.md"
    write_keyword_trio_file "$TMP_REPO/skills/autospec-listen/opencode/agent.md"
    write_keyword_trio_file "$TMP_REPO/skills/autospec-listen/codex/prompt.md"
}

run_keyword_check() {
    (
        cd "$TMP_REPO"
        CHECK_BODY="$(awk '
            /^check_keyword_routing_section\(\) \{/ {capture=1}
            capture {print}
            capture && /^\}/ {exit}
        ' "$VALIDATE_SH")"
        eval "$CHECK_BODY"
        info() { printf 'validate: %s\n' "$*"; }
        fail() { printf 'validate: FAIL — %s\n' "$*" >&2; exit 1; }
        check_keyword_routing_section
    )
}

@test "check_keyword_routing_section passes when autospec-listen trio and classifier literals are aligned" {
    write_complete_keyword_trio

    run run_keyword_check

    [ "$status" -eq 0 ]
    [[ "$output" == *"keyword-routing: autospec-listen"* ]]
}

@test "check_keyword_routing_section fails when a trio file is missing the keyword heading" {
    write_complete_keyword_trio
    sed -i 's/^## Keyword auto-routing$/## Different routing/' "$TMP_REPO/skills/autospec-listen/codex/prompt.md"

    run run_keyword_check

    [ "$status" -ne 0 ]
    [[ "$output" == *"autospec-listen: codex/prompt.md missing '## Keyword auto-routing' section"* ]]
}

@test "check_keyword_routing_section fails when classifier drift drops explore-confirm" {
    write_complete_keyword_trio
    printf '#!/usr/bin/env bash\n# autospec fix imperative\n# post_approval_execution_ready\n# plan_exit_ready\n' >"$TMP_REPO/scripts/listener-match.sh"

    run run_keyword_check

    [ "$status" -ne 0 ]
    [[ "$output" == *"scripts/listener-match.sh missing 'explore-confirm' gate literal"* ]]
}
