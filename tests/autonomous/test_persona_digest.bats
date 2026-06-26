#!/usr/bin/env bats
# tests/autonomous/test_persona_digest.bats — TDD contract for F6:
# digest persona/priorities blocks + autospec:recalibrate-persona control label.
#
# Coverage:
#   1. Digest renders a persona block with last-refresh, per-dimension confidence,
#      and calibration-agreement % when the persona file is present.
#   2. Digest renders a "not yet built" notice when the persona file is absent
#      (fail-soft — digest must not fail).
#   3. Digest renders a priorities block listing active priorities.
#   4. Digest renders biased filed work (DIRECTIVE:/PRIORITY_ISSUE: entries).
#   5. autospec:recalibrate-persona label emits DECISION:persona-recalibrate
#      and writes a persona-recalibrate.flag.
#   6. Existing digest sections (header, drift stub) remain present after F6 additions.
#
# Engineering notes:
#   - All fixtures written to real temp files (macOS bash 3.2: [ -f <(…) ] always false).
#   - .autospec/ is gitignored; all state materialised under TMP at runtime.
#   - gh binary mocked as a real subprocess via $TMP/bin/gh.
#   - No RETURN traps; no process substitutions with [ -f ].
#   - printf with format starting '-' uses '-- ' prefix (bash 3.2 gotcha).
#   - jq capture()/== — no dynamic interpolation into test().

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
LOOP_LIB="$REPO_ROOT/scripts/lib/autospec-loop.sh"
CONTROL_CHANNEL="$REPO_ROOT/scripts/autonomous-control-channel.sh"

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

setup() {
    TMP="$(mktemp -d -t test-persona-digest.XXXXXX)"

    # Fake .autospec state dir under TMP (not ~/.autospec).
    AUTOSPEC_STATE_DIR="$TMP/autospec-state"
    mkdir -p "$AUTOSPEC_STATE_DIR"

    # Fake repo dir with a scripts/ subdir so sdir logic resolves cleanly.
    FAKE_REPO="$TMP/repo"
    mkdir -p "$FAKE_REPO/scripts"
    # .autospec/ inside fake repo.
    mkdir -p "$FAKE_REPO/.autospec"

    # Persona fixture (well-shaped effective persona).
    PERSONA_FILE="$AUTOSPEC_STATE_DIR/operator-persona.md"

    # Priorities fixture.
    PRIORITIES_FILE="$AUTOSPEC_STATE_DIR/autonomous-priorities.md"

    # Fake bin dir for gh stub.
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
echo "[]"
EOF
    chmod +x "$TMP/bin/gh"

    export PATH="$TMP/bin:$PATH"
    export AUTOSPEC_CONTROL_STATE_DIR="$AUTOSPEC_STATE_DIR"
}

teardown() {
    rm -rf "$TMP"
}

# Write a well-shaped persona file with per-dimension confidence.
_write_persona() {
    cat > "$PERSONA_FILE" <<'EOF'
# Operator persona

## Decision style

Favors correctness over speed.

## Risk tolerance

Conservative; lock-step discipline.

## Confidence (per dimension)

_Derived from 3 source(s) (global=2, overlay=1)._

- Decision style: high
- Risk tolerance: high
EOF
}

# Helper: call _conductor_maybe_write_digest via a subshell sourcing LOOP_LIB.
_call_digest() {
    local no_digest="${1:-0}"
    local last_day="${2:-}"
    local sdir="${3:-$FAKE_REPO/scripts}"
    local repo="${4:-}"
    local dry="${5:-0}"

    bash -c "
        source '$LOOP_LIB'
        AUTOSPEC_PERSONA_FILE='$PERSONA_FILE' \
        AUTOSPEC_PRIORITIES_FILE='$PRIORITIES_FILE' \
        _conductor_maybe_write_digest '$no_digest' '$last_day' '$sdir' '$repo' '$dry'
    " 2>/dev/null
}

# Read the digest file produced inside FAKE_REPO.
_digest_file() {
    printf '%s' "$FAKE_REPO/.autospec/autonomous-digest.md"
}

# ---------------------------------------------------------------------------
# persona block — file present
# ---------------------------------------------------------------------------

@test "digest persona block: last-refresh line is present when persona file exists" {
    _write_persona

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    [ -f "$(_digest_file)" ]
    grep -q 'Last refresh' "$(_digest_file)"
}

@test "digest persona block: per-dimension confidence lines are present" {
    _write_persona

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    grep -q 'Per-dimension confidence' "$(_digest_file)"
    grep -q 'Decision style' "$(_digest_file)"
    grep -q 'Risk tolerance' "$(_digest_file)"
}

@test "digest persona block: calibration-agreement % is present" {
    _write_persona

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    grep -q 'Calibration-agreement' "$(_digest_file)"
    # Two dims both high → 100%.
    grep -q '100%' "$(_digest_file)"
}

# ---------------------------------------------------------------------------
# persona block — file absent (fail-soft)
# ---------------------------------------------------------------------------

@test "digest persona block: renders 'not yet built' when persona file is absent" {
    # Ensure persona file does NOT exist.
    rm -f "$PERSONA_FILE"

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    [ -f "$(_digest_file)" ]
    grep -q 'not yet built' "$(_digest_file)"
}

@test "digest: renders successfully even when persona file is absent" {
    rm -f "$PERSONA_FILE"

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    # Digest must exist and contain the standard header.
    [ -f "$(_digest_file)" ]
    grep -q 'autospec-autonomous daily digest' "$(_digest_file)"
}

# ---------------------------------------------------------------------------
# priorities block
# ---------------------------------------------------------------------------

@test "digest priorities block: active priorities appear in digest" {
    _write_persona
    cat > "$PRIORITIES_FILE" <<'EOF'
- reduce CI flakiness
- ship persona calibration by Q3
EOF

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    grep -q 'Active priorities' "$(_digest_file)"
    grep -q 'reduce CI flakiness' "$(_digest_file)"
    grep -q 'ship persona calibration' "$(_digest_file)"
}

@test "digest priorities block: biased filed work (DIRECTIVE/PRIORITY_ISSUE) appears" {
    _write_persona
    cat > "$PRIORITIES_FILE" <<'EOF'
- reduce CI flakiness
DIRECTIVE:focus on reducing CI flakiness this sprint
PRIORITY_ISSUE:99
EOF

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    grep -q 'Biased filed work' "$(_digest_file)"
    grep -q 'DIRECTIVE:' "$(_digest_file)"
    grep -q 'PRIORITY_ISSUE:99' "$(_digest_file)"
}

@test "digest priorities block: omitted when priorities file is absent" {
    _write_persona
    rm -f "$PRIORITIES_FILE"

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    # Digest must render but priorities section must not appear.
    [ -f "$(_digest_file)" ]
    run grep -c 'Active priorities' "$(_digest_file)"
    [ "$output" = "0" ]
}

# ---------------------------------------------------------------------------
# existing digest sections still present
# ---------------------------------------------------------------------------

@test "digest: standard header and conductor line still present after F6 additions" {
    _write_persona

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "myorg/myrepo" 0

    grep -q 'autospec-autonomous daily digest' "$(_digest_file)"
    grep -q 'autospec_conductor_run' "$(_digest_file)"
    grep -q 'myorg/myrepo' "$(_digest_file)"
}

@test "digest: Phase-1 footer line still present" {
    _write_persona

    _call_digest 0 "1970-01-01" "$FAKE_REPO/scripts" "" 0

    grep -q 'Generated by autospec-autonomous' "$(_digest_file)"
}

# ---------------------------------------------------------------------------
# autospec:recalibrate-persona control label
# ---------------------------------------------------------------------------

@test "recalibrate-persona label: emits DECISION:persona-recalibrate" {
    # gh stub returns one issue for autospec:recalibrate-persona.
    local fixture_file="$TMP/recal_fixture.json"
    printf '[{"number":42,"title":"recalibrate","body":""}]\n' > "$fixture_file"

    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
found_label=""
while [ "\$#" -gt 0 ]; do
    if [ "\$1" = "--label" ] && [ "\$2" = "autospec:recalibrate-persona" ]; then
        found_label="\$2"
    fi
    shift
done
if [ -n "\$found_label" ]; then
    cat "$fixture_file"
else
    echo "[]"
fi
EOF
    chmod +x "$TMP/bin/gh"

    run bash "$CONTROL_CHANNEL" --state-dir "$AUTOSPEC_STATE_DIR"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'DECISION:persona-recalibrate'
}

@test "recalibrate-persona label: writes persona-recalibrate.flag" {
    local fixture_file="$TMP/recal_fixture2.json"
    printf '[{"number":7,"title":"recalibrate","body":""}]\n' > "$fixture_file"

    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
found_label=""
while [ "\$#" -gt 0 ]; do
    if [ "\$1" = "--label" ] && [ "\$2" = "autospec:recalibrate-persona" ]; then
        found_label="\$2"
    fi
    shift
done
if [ -n "\$found_label" ]; then
    cat "$fixture_file"
else
    echo "[]"
fi
EOF
    chmod +x "$TMP/bin/gh"

    bash "$CONTROL_CHANNEL" --state-dir "$AUTOSPEC_STATE_DIR"

    [ -f "$AUTOSPEC_STATE_DIR/persona-recalibrate.flag" ]
    grep -q 'recalibrate' "$AUTOSPEC_STATE_DIR/persona-recalibrate.flag"
}

@test "recalibrate-persona label absent: no DECISION:persona-recalibrate emitted" {
    # Default gh stub returns [].
    run bash "$CONTROL_CHANNEL" --state-dir "$AUTOSPEC_STATE_DIR"
    [ "$status" -eq 0 ]
    run printf '%s\n' "$output"
    # Should not contain persona-recalibrate decision.
    run grep -c 'persona-recalibrate' "$AUTOSPEC_STATE_DIR/persona-recalibrate.flag" 2>/dev/null || true
    # Flag file must NOT exist.
    [ ! -f "$AUTOSPEC_STATE_DIR/persona-recalibrate.flag" ]
}
