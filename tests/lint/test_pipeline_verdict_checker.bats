#!/usr/bin/env bats
#
# Self-enforcement for scripts/lint-pipeline-verdict.sh (issue #3716).
#
# In a pipeline the exit status you get is not the exit status you meant:
# `cmd | filter && echo ok` reads the status of the filter, so the verdict
# prints `ok` no matter what `cmd` did. The ratchet counts those sites per
# file against an allowlist; the count may only shrink.

setup() {
    REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd -P)"
    CHECKER="${REPO_ROOT}/scripts/lint-pipeline-verdict.sh"
    TMP_ROOT="$(mktemp -d)"
    mkdir -p "${TMP_ROOT}/scripts" "${TMP_ROOT}/tests/fixtures"
    ALLOWLIST="${TMP_ROOT}/tests/fixtures/pipeline-verdict-allowlist.txt"
    : > "${ALLOWLIST}"
}

teardown() {
    rm -rf "${TMP_ROOT}"
}

# Fixture bodies are assembled with printf, not heredocs, so no line in this
# file ever begins with a bare `@test` token: Bats' own preprocessor extracts
# those wherever they appear, heredoc or not.

write_issue_pattern() {
    # The exact shape from the issue: verdict derived from the pipeline's tail.
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'cargo +1.91.0 fmt --check 2>&1 | head -2 && echo "  fmt clean"' \
        > "$1"
}

write_branched_rewrite() {
    # Branch on the gate's own status: the verdict is unreachable on failure.
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'if cargo fmt --check >/dev/null 2>&1; then' \
        '  echo "fmt clean"' \
        'else' \
        '  echo "DIRTY"' \
        'fi' \
        > "$1"
}

write_pipefail() {
    # pipefail makes the pipeline itself carry the gate's status.
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'set -euo pipefail' \
        'cargo fmt --check 2>&1 | head -2 && echo "fmt clean"' \
        > "$1"
}

write_quoted_data() {
    # Quoted strings are data, never code.
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'echo "cmd | filter && echo ok is a defect"' \
        > "$1"
}

write_heredoc_payload() {
    # Heredoc payloads are data, never code.
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'cat <<PAYLOAD' \
        'cmd | filter && echo ok' \
        'PAYLOAD' \
        > "$1"
}

write_case_pattern() {
    # `|` in a case pattern is alternation, not a pipeline.
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'case "$1" in' \
        '  --repo-root|--repo) [ "$#" -ge 2 ] || die "$1 requires a value" ;;' \
        'esac' \
        > "$1"
}

write_two_sites() {
    printf '%s\n' \
        '#!/usr/bin/env bash' \
        'a | head && echo one' \
        'b | tail || echo two' \
        > "$1"
}

@test "the shipped allowlist keeps the real repository scan green" {
    run bash "${CHECKER}" --root "${REPO_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a pipeline-tail verdict in an unlisted file is a blocking finding" {
    write_issue_pattern "${TMP_ROOT}/scripts/sample.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"PIPELINE_VERDICT:scripts/sample.sh"* ]]
}

@test "the top-level scripts of the repository are in scope" {
    write_issue_pattern "${TMP_ROOT}/install.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"install.sh"* ]]
}

@test "branching on the gate's own status is not a site" {
    write_branched_rewrite "${TMP_ROOT}/scripts/sample.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a script under set -euo pipefail is exempt" {
    write_pipefail "${TMP_ROOT}/scripts/sample.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a quoted string that contains the pattern is not a site" {
    write_quoted_data "${TMP_ROOT}/scripts/sample.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a heredoc payload that contains the pattern is not a site" {
    write_heredoc_payload "${TMP_ROOT}/scripts/sample.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a case-pattern alternative is not a pipeline" {
    write_case_pattern "${TMP_ROOT}/scripts/sample.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "exceeding a file's allowlisted count is a blocking finding" {
    write_two_sites "${TMP_ROOT}/scripts/sample.sh"
    echo "scripts/sample.sh 1" > "${ALLOWLIST}"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"scripts/sample.sh"* ]]
}

@test "matching the allowlisted count passes and one more site fails" {
    write_issue_pattern "${TMP_ROOT}/scripts/sample.sh"
    echo "scripts/sample.sh 1" > "${ALLOWLIST}"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]

    write_two_sites "${TMP_ROOT}/scripts/sample.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -ne 0 ]
}

@test "a count below the allowlist entry passes the ratchet" {
    write_issue_pattern "${TMP_ROOT}/scripts/sample.sh"
    echo "scripts/sample.sh 5" > "${ALLOWLIST}"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "--list prints every site as path:line" {
    write_issue_pattern "${TMP_ROOT}/scripts/sample.sh"
    run bash "${CHECKER}" --root "${TMP_ROOT}" --list
    [ "$status" -eq 0 ]
    [[ "$output" == "scripts/sample.sh:2" ]]
}
