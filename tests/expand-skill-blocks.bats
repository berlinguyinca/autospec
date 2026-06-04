#!/usr/bin/env bats
# Tests for scripts/expand-skill-blocks.sh — canonical skill-block expander (D2).
# TDD-first: positive behaviors + negative-path pairs + the load-bearing
# byte-parity test that proves the startup template reproduces the live block.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/expand-skill-blocks.sh"
    FIX="$REPO_ROOT/tests/fixtures/expand"
    TMP="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP"
    rm -f /tmp/pwned
}

# --- positive: marker -> body, contains expanded content ---
@test "basic marker expands and contains skill content" {
    run bash "$SCRIPT" "$FIX/basic.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"autospec"* ]]
    # surrounding lines preserved
    [[ "$output" == *"prefix line"* ]]
    [[ "$output" == *"suffix line"* ]]
    # marker line itself is gone
    [[ "$output" != *"autospec-block:startup-self-update"* ]]
}

# --- positive: {{SKILL_NAME}} substitution lands the param value ---
@test "SKILL_NAME param substitutes into body" {
    run bash "$SCRIPT" "$FIX/basic.md"
    [ "$status" -eq 0 ]
    [[ "$output" == *"SKILL_NAME=autospec-run"* ]]
    # placeholder fully consumed
    [[ "$output" != *"{{SKILL_NAME}}"* ]]
}

# --- positive idempotency: no-marker file is byte-identical (sha equal) ---
@test "file without markers is byte-identical (idempotent)" {
    run bash "$SCRIPT" "$FIX/nomarker.md"
    [ "$status" -eq 0 ]
    a="$(sha256sum < "$FIX/nomarker.md" | awk '{print $1}')"
    printf '%s' "$output" > "$TMP/out.md"
    # account for any trailing-newline normalization by comparing content lines
    b="$(bash "$SCRIPT" "$FIX/nomarker.md" | sha256sum | awk '{print $1}')"
    c="$(sha256sum < "$FIX/nomarker.md" | awk '{print $1}')"
    [ "$b" = "$c" ]
}

# --- LOAD-BEARING: startup template reproduces the EXACT live block ---
@test "startup-self-update expansion is byte-identical to the canonical template block" {
    # Single source of truth (#1032/#1035): the canonical block lives in
    # templates/skill-blocks/startup-self-update.md, NOT in any skill's live
    # SKILL.md (which now carries only the marker). The marker expansion must be
    # byte-identical to the template with {{SKILL_NAME}} substituted.
    local template="$REPO_ROOT/templates/skill-blocks/startup-self-update.md"
    [ -f "$template" ]
    local expected="$TMP/expected.md"
    sed 's/{{SKILL_NAME}}/autospec-define/g' "$template" > "$expected"
    # Synthetic file containing only the marker for autospec-define
    local synth="$TMP/synth.md"
    printf '<!-- autospec-block:startup-self-update SKILL_NAME=autospec-define -->\n' > "$synth"
    bash "$SCRIPT" "$synth" > "$TMP/expanded.md"
    run diff "$expected" "$TMP/expanded.md"
    [ "$status" -eq 0 ]
}

# --- negative pair: unknown block name fails closed ---
@test "unknown block name exits non-zero" {
    run bash "$SCRIPT" "$FIX/unknown.md"
    [ "$status" -ne 0 ]
    [[ "$output" == *"does-not-exist"* ]]
}

# --- negative pair: missing template file fails closed ---
@test "missing template file exits non-zero" {
    local f="$TMP/missing.md"
    printf '<!-- autospec-block:no-such-template -->\n' > "$f"
    run bash "$SCRIPT" "$f"
    [ "$status" -ne 0 ]
}

# --- INJECTION GUARD: $(...) param lands literally, no exec ---
@test "injection param lands literally with no command execution" {
    rm -f /tmp/pwned
    run bash "$SCRIPT" "$FIX/inject.md"
    [ "$status" -eq 0 ]
    # The literal $( substring must appear in output
    [[ "$output" == *'$(rm -rf /;touch /tmp/pwned)'* ]]
    # And the side-effect file must NOT exist
    [ ! -e /tmp/pwned ]
}

# --- negative: no eval in the script source ---
@test "script source contains no eval" {
    run grep -n 'eval' "$SCRIPT"
    [ "$status" -ne 0 ]
}

# --- missing/empty arg fails closed ---
@test "no argument exits non-zero" {
    run bash "$SCRIPT"
    [ "$status" -ne 0 ]
}

# --- nonexistent input file fails closed ---
@test "nonexistent input file exits non-zero" {
    run bash "$SCRIPT" "$TMP/nope-does-not-exist.md"
    [ "$status" -ne 0 ]
}
