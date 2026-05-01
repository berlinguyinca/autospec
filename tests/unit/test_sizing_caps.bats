#!/usr/bin/env bats
# tests/unit/test_sizing_caps.bats — exercises scripts/sizing-check.sh
# against positive (compliant) and negative (over-cap) fixture bodies for
# each of the three §3.4 caps:
#   - Body ≤400 words
#   - Implementation outline ≤30 lines
#   - Files touched ≤3

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SIZING="$REPO_ROOT/scripts/sizing-check.sh"
}

# ---- helpers -------------------------------------------------------------

# Generate a body with N "word" tokens — used to drive the body-words cap.
gen_words() {
    n="$1"
    i=0
    while [ "$i" -lt "$n" ]; do
        printf 'word '
        i=$((i + 1))
    done
}

# Generate an outline section with N bullet lines, each touching a unique
# file path (so the file-cap is not the trigger — we are testing the
# line-count cap).
gen_outline_lines() {
    n="$1"
    printf '## Implementation outline\n'
    i=0
    while [ "$i" -lt "$n" ]; do
        printf '- `path/file_%d.txt` — note line.\n' "$i"
        i=$((i + 1))
    done
}

# Generate an outline section with N distinct file paths (one bullet each).
gen_outline_files() {
    n="$1"
    gen_outline_lines "$n"
}

# ---- body-words cap ------------------------------------------------------

@test "body-words: 100 words passes (under cap)" {
    body="$(gen_words 100)"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -eq 0 ]
}

@test "body-words: 400 words passes (at cap)" {
    body="$(gen_words 400)"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -eq 0 ]
}

@test "body-words: 401 words fails (over cap)" {
    body="$(gen_words 401)"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -ne 0 ]
    [[ "$output" == *body-words* ]]
}

# ---- outline-lines cap ---------------------------------------------------

@test "outline-lines: 10 bullets passes (under cap)" {
    body="$(gen_outline_lines 10)"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    # Note: 10 bullets = 10 outline lines, plus 10 distinct files (over the
    # file cap). To isolate the line cap, ensure file count ≤ 3 by reusing
    # one path. Re-render with shared paths.
    body="$(printf '## Implementation outline\n'; i=0; while [ "$i" -lt 10 ]; do printf -- '- `path/a.txt` — note line.\n'; i=$((i + 1)); done)"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -eq 0 ]
}

@test "outline-lines: 30 bullets passes (at cap)" {
    body="$(printf '## Implementation outline\n'; i=0; while [ "$i" -lt 30 ]; do printf -- '- `path/a.txt` — n.\n'; i=$((i + 1)); done)"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -eq 0 ]
}

@test "outline-lines: 31 bullets fails (over cap)" {
    body="$(printf '## Implementation outline\n'; i=0; while [ "$i" -lt 31 ]; do printf -- '- `path/a.txt` — n.\n'; i=$((i + 1)); done)"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -ne 0 ]
    [[ "$output" == *outline-lines* ]]
}

# ---- outline-files cap ---------------------------------------------------

@test "outline-files: 3 distinct files passes (at cap)" {
    body="$(printf '## Implementation outline\n- \`a.txt\` — n.\n- \`b.txt\` — n.\n- \`c.txt\` — n.\n')"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -eq 0 ]
}

@test "outline-files: 4 distinct files fails (over cap)" {
    body="$(printf '## Implementation outline\n- \`a.txt\` — n.\n- \`b.txt\` — n.\n- \`c.txt\` — n.\n- \`d.txt\` — n.\n')"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -ne 0 ]
    [[ "$output" == *outline-files* ]]
}

@test "outline-files: 5 bullets all referencing the same path passes" {
    body="$(printf '## Implementation outline\n- \`shared.txt\` — n.\n- \`shared.txt\` — n.\n- \`shared.txt\` — n.\n- \`shared.txt\` — n.\n- \`shared.txt\` — n.\n')"
    run bash -c "printf '%s\n' \"$body\" | '$SIZING'"
    [ "$status" -eq 0 ]
}
