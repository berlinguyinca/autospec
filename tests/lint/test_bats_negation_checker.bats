#!/usr/bin/env bats
#
# Self-enforcement for scripts/lint-bats-negations.sh (issue #3091).
#




setup() {
    REPO_ROOT="$(cd "${BATS_TEST_DIRNAME}/../.." && pwd -P)"
    CHECKER="${REPO_ROOT}/scripts/lint-bats-negations.sh"
    TMP_ROOT="$(mktemp -d)"
    mkdir -p "${TMP_ROOT}/tests/fixtures"
    ALLOWLIST="${TMP_ROOT}/tests/fixtures/bats-negation-allowlist.txt"
    : > "${ALLOWLIST}"
}

teardown() {
    rm -rf "${TMP_ROOT}"
}

# Fixture bodies are assembled with printf, not heredocs, so no line in this
# file ever begins with a bare `@test` token: Bats' own preprocessor extracts
# those wherever they appear, heredoc or not.

write_one_midbody() {
    printf '%s\n' \
        '@test "mid-body negation cannot fail" {' \
        '  ! grep -q absent /dev/null' \
        '  [ 1 -eq 1 ]' \
        '}' > "$1"
}

write_two_midbody() {
    printf '%s\n' \
        '@test "two mid-body negations" {' \
        '  ! grep -q absent /dev/null' \
        '  ! grep -q missing /dev/null' \
        '  [ 1 -eq 1 ]' \
        '}' > "$1"
}

write_run_bang() {
    printf '%s\n' \
        '@test "run ! form is observable" {' \
        '  run ! grep -q absent /dev/null' \
        '  [ "$status" -eq 0 ]' \
        '}' > "$1"
}

write_final_negation() {
    printf '%s\n' \
        '@test "negation as the final statement can fail" {' \
        '  [ 1 -eq 1 ]' \
        '  ! grep -q absent /dev/null' \
        '}' > "$1"
}

write_heredoc_payload() {
    printf '%s\n' \
        '@test "heredoc payload is data, not an assertion" {' \
        '  cat > payload.txt <<PAYLOAD' \
        '! this line is payload, not a negated command' \
        'PAYLOAD' \
        '  [ -s payload.txt ]' \
        '}' > "$1"
}

@test "the shipped allowlist keeps the real repository scan green" {
    run bash "${CHECKER}" --root "${REPO_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a mid-body negation in an unlisted file is a blocking finding" {
    write_one_midbody "${TMP_ROOT}/tests/sample.bats"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"tests/sample.bats"* ]]
}

@test "the same assertion rewritten as run ! is not a site" {
    write_run_bang "${TMP_ROOT}/tests/sample.bats"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a negation that ends its block is not a site" {
    write_final_negation "${TMP_ROOT}/tests/sample.bats"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "a heredoc payload inside a test block is not a site" {
    write_heredoc_payload "${TMP_ROOT}/tests/sample.bats"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "exceeding a file's allowlisted count is a blocking finding" {
    write_two_midbody "${TMP_ROOT}/tests/sample.bats"
    echo "tests/sample.bats 1" > "${ALLOWLIST}"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"tests/sample.bats"* ]]
}

@test "matching the allowlisted count passes and one more negation fails" {
    write_one_midbody "${TMP_ROOT}/tests/sample.bats"
    echo "tests/sample.bats 1" > "${ALLOWLIST}"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]

    write_two_midbody "${TMP_ROOT}/tests/sample.bats"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -ne 0 ]
}

@test "a count below the allowlist entry passes the ratchet" {
    write_one_midbody "${TMP_ROOT}/tests/sample.bats"
    echo "tests/sample.bats 5" > "${ALLOWLIST}"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -eq 0 ]
}

@test "the scan reaches skills/<skill>/tests/*.bats, not only tests/" {
    mkdir -p "${TMP_ROOT}/skills/demo-skill/tests"
    write_one_midbody "${TMP_ROOT}/skills/demo-skill/tests/sample.bats"
    run bash "${CHECKER}" --root "${TMP_ROOT}"
    [ "$status" -ne 0 ]
    [[ "$output" == *"skills/demo-skill/tests/sample.bats"* ]]
}
