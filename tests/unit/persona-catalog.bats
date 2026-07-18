#!/usr/bin/env bats
# tests/unit/persona-catalog.bats — merged bundled/user persona archetype catalog
#
# Coverage:
#   - list returns bundled + user-state ids
#   - load returns front matter and body for bundled ids
#   - user-state shadows bundled ids on collision
#   - missing ids fail clearly
#   - malformed front matter is skipped with a warning

if [ -z "${BATS_TEST_DIRNAME:-}" ]; then
    exec bats "$0" "$@"
fi

bats_require_minimum_version 1.5.0

SCRIPT_DIR="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
PERSONA_CATALOG="$SCRIPT_DIR/scripts/persona-catalog.sh"

setup() {
    TMP="$(mktemp -d -t test-persona-catalog.XXXXXX)"
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec/personas"
}

teardown() {
    rm -rf "$TMP"
}

write_user_persona() {
    local id="$1"
    local body="$2"
    cat > "$HOME/.autospec/personas/${id}.md" <<EOF
---
id: ${id}
title: ${id} user override
---
${body}
EOF
}

run_catalog() {
    run bash "$PERSONA_CATALOG" "$@"
}

@test "list returns union of bundled and user-state ids" {
    write_user_persona "site-reliability" "User-only persona."

    run_catalog list

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -qx "security-hardener"
    printf '%s\n' "$output" | grep -qx "site-reliability"
}

@test "load prints front matter and body for a bundled id" {
    run_catalog load security-hardener

    [ "$status" -eq 0 ]
    [ "${lines[0]}" = "---" ]
    printf '%s\n' "$output" | grep -q "^id: security-hardener$"
    printf '%s\n' "$output" | grep -q "Security Hardener"
}

@test "user-state id shadows bundled id on load and logs the decision" {
    write_user_persona "security-hardener" "USER-SHADOW-BODY"

    run_catalog load security-hardener

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "USER-SHADOW-BODY"
    printf '%s\n' "$output" | grep -q "persona-catalog: user-state shadows bundled id: security-hardener"
}

@test "load of a missing id fails without printing a persona body" {
    run_catalog load does-not-exist

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "persona-catalog: id not found: does-not-exist"
}

@test "malformed front matter is skipped with a warning during list and load" {
    cat > "$HOME/.autospec/personas/malformed.md" <<'EOF'
id: malformed
missing opening fence
EOF

    run_catalog list

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "persona-catalog: warning: malformed front matter:"
    if printf '%s\n' "$output" | grep -qx "malformed"; then
        echo "malformed id should have been skipped" >&2
        return 1
    fi

    run_catalog load malformed

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q "persona-catalog: warning: malformed front matter:"
    printf '%s\n' "$output" | grep -q "persona-catalog: id not found: malformed"
}
