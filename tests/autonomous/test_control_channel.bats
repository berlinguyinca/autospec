#!/usr/bin/env bats
# tests/autonomous/test_control_channel.bats — TDD contract for
# scripts/autonomous-control-channel.sh
#
# Design:
#   - Stubs gh as a PATH-resident subprocess (writes real temp files, not
#     process substitutions — macOS bash 3.2 [ -f <(...) ] is false).
#   - All state goes under $TMP to avoid touching real ~/.autospec.
#   - Each test has its own gh stub that emits controlled fixture JSON.

SCRIPT_DIR="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
CONTROL_CHANNEL="$SCRIPT_DIR/scripts/autonomous-control-channel.sh"

setup() {
    TMP="$(mktemp -d -t ctrl-chan.XXXXXX)"
    export AUTOSPEC_CONTROL_STATE_DIR="$TMP/state"
    mkdir -p "$TMP/bin" "$TMP/state"

    # Put fake-gh directory first in PATH; each test writes its own gh stub.
    export PATH="$TMP/bin:$PATH"

    # Default gh stub: returns empty array for all labels (no control issues).
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
echo "[]"
EOF
    chmod +x "$TMP/bin/gh"
}

teardown() {
    rm -rf "$TMP"
}

# Helper: write a gh stub that returns FIXTURE_JSON for any invocation.
make_gh_stub() {
    local fixture_file="$1"   # path to a file containing the JSON to emit
    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
cat "$fixture_file"
EOF
    chmod +x "$TMP/bin/gh"
}

# Helper: write a gh stub that maps --label VALUE to a specific fixture file.
# Usage: make_gh_label_stub "autospec:stop" /path/to/file.json
make_gh_label_stub() {
    local target_label="$1"
    local fixture_file="$2"
    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
# Scan args for --label <value>
found_label=""
while [ "\$#" -gt 0 ]; do
    if [ "\$1" = "--label" ] && [ "\$2" = "$target_label" ]; then
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
}

# ---------------------------------------------------------------------------
# Basic: no control labels → no decisions
# ---------------------------------------------------------------------------

@test "no reserved labels → exits 0 with no output" {
    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ---------------------------------------------------------------------------
# autospec:stop → DECISION:graceful-stop
# ---------------------------------------------------------------------------

@test "autospec:stop label → DECISION:graceful-stop" {
    local fixture="$TMP/stop.json"
    printf '[{"number":42,"title":"stop the run","body":"operator request"}]' > "$fixture"
    make_gh_label_stub "autospec:stop" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:graceful-stop$"
}

# ---------------------------------------------------------------------------
# autospec:pause → DECISION:pause + flag written
# ---------------------------------------------------------------------------

@test "autospec:pause label → DECISION:pause" {
    local fixture="$TMP/pause.json"
    printf '[{"number":43,"title":"pause","body":"taking a break"}]' > "$fixture"
    make_gh_label_stub "autospec:pause" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:pause$"
}

@test "autospec:pause writes autonomous-pause.flag" {
    local fixture="$TMP/pause.json"
    printf '[{"number":43,"title":"pause","body":"pause body"}]' > "$fixture"
    make_gh_label_stub "autospec:pause" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    [ -f "$TMP/state/autonomous-pause.flag" ]
    # Flag must contain "pause" on first line
    first_line="$(head -1 "$TMP/state/autonomous-pause.flag")"
    [ "$first_line" = "pause" ]
}

@test "autospec:pause flag write is idempotent (second call overwrites cleanly)" {
    local fixture="$TMP/pause.json"
    printf '[{"number":43,"title":"pause","body":"pause body"}]' > "$fixture"
    make_gh_label_stub "autospec:pause" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    [ -f "$TMP/state/autonomous-pause.flag" ]
}

# ---------------------------------------------------------------------------
# autospec:steer → DECISION:steer + DIRECTIVE + STEER_ISSUE + STEER_REMOVE_LABEL
# ---------------------------------------------------------------------------

@test "autospec:steer label → DECISION:steer" {
    local fixture="$TMP/steer.json"
    printf '[{"number":99,"title":"steer me","body":"pivot to auth module"}]' > "$fixture"
    make_gh_label_stub "autospec:steer" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:steer$"
}

@test "autospec:steer emits STEER_ISSUE with issue number" {
    local fixture="$TMP/steer.json"
    printf '[{"number":99,"title":"steer me","body":"pivot to auth module"}]' > "$fixture"
    make_gh_label_stub "autospec:steer" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^STEER_ISSUE:99$"
}

@test "autospec:steer emits DIRECTIVE containing issue body" {
    local fixture="$TMP/steer.json"
    printf '[{"number":99,"title":"steer me","body":"focus on the payment module"}]' > "$fixture"
    make_gh_label_stub "autospec:steer" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DIRECTIVE:.*focus on the payment module"
}

@test "autospec:steer emits STEER_REMOVE_LABEL with issue number" {
    local fixture="$TMP/steer.json"
    printf '[{"number":99,"title":"steer me","body":"do the thing"}]' > "$fixture"
    make_gh_label_stub "autospec:steer" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^STEER_REMOVE_LABEL:99$"
}

@test "autospec:steer body with multiline content is emitted as a single DIRECTIVE line" {
    local fixture="$TMP/steer.json"
    # Multi-line body — jq will encode the newline in the JSON string
    local body_json
    body_json='[{"number":7,"title":"steer","body":"line one\nline two\nline three"}]'
    printf '%s' "$body_json" > "$fixture"
    make_gh_label_stub "autospec:steer" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    # DIRECTIVE must be exactly one line
    directive_count="$(printf '%s\n' "$output" | grep -c "^DIRECTIVE:" || true)"
    [ "$directive_count" -eq 1 ]
}

# ---------------------------------------------------------------------------
# autospec:priority → DECISION:priority + PRIORITY_ISSUE
# ---------------------------------------------------------------------------

@test "autospec:priority label → DECISION:priority" {
    local fixture="$TMP/priority.json"
    printf '[{"number":77,"title":"urgent fix","body":"bump this"}]' > "$fixture"
    make_gh_label_stub "autospec:priority" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:priority$"
}

@test "autospec:priority emits PRIORITY_ISSUE with issue number" {
    local fixture="$TMP/priority.json"
    printf '[{"number":77,"title":"urgent fix","body":"bump this"}]' > "$fixture"
    make_gh_label_stub "autospec:priority" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^PRIORITY_ISSUE:77$"
}

# ---------------------------------------------------------------------------
# jq safety: metachar-laden label/body parses without error or false match
# ---------------------------------------------------------------------------

@test "body with regex metacharacters parses safely (no injection via jq test)" {
    local fixture="$TMP/steer_meta.json"
    # Body contains: dots, asterisks, brackets, parens — all jq regex metachars
    local meta_body='mac.local user: [a-z]+ (foo|bar) pricing.*now'
    # Build valid JSON using printf with jq to encode the body safely
    printf '[{"number":55,"title":"metachar steer","body":"%s"}]' "$meta_body" > "$fixture"
    make_gh_label_stub "autospec:steer" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:steer$"
    printf '%s\n' "$output" | grep -q "^STEER_ISSUE:55$"
    # DIRECTIVE line must be present and contain the metachar content (no crash)
    printf '%s\n' "$output" | grep -q "^DIRECTIVE:"
}

@test "stop body with regex metacharacters does not cause jq crash" {
    local fixture="$TMP/stop_meta.json"
    printf '[{"number":10,"title":"stop(now)","body":"halt [immediately] for mac.local+user.*"}]' > "$fixture"
    make_gh_label_stub "autospec:stop" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:graceful-stop$"
}

# ---------------------------------------------------------------------------
# Severity ordering: stop appears before pause before steer before priority
# ---------------------------------------------------------------------------

@test "severity order: stop decision appears before priority in output" {
    # Both stop and priority labels have issues
    local stop_fixture="$TMP/stop2.json"
    local priority_fixture="$TMP/priority2.json"
    printf '[{"number":1,"title":"stop","body":"stop"}]' > "$stop_fixture"
    printf '[{"number":2,"title":"prio","body":"prio"}]' > "$priority_fixture"

    # gh stub that returns issues for both labels
    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
label=""
while [ "\$#" -gt 0 ]; do
    if [ "\$1" = "--label" ]; then
        label="\$2"
    fi
    shift
done
case "\$label" in
    "autospec:stop")     cat "$stop_fixture" ;;
    "autospec:priority") cat "$priority_fixture" ;;
    *)                   echo "[]" ;;
esac
EOF
    chmod +x "$TMP/bin/gh"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]

    # stop must appear on a line BEFORE priority in the output
    stop_line="$(printf '%s\n' "$output" | grep -n "^DECISION:graceful-stop$" | head -1 | cut -d: -f1)"
    prio_line="$(printf '%s\n' "$output" | grep -n "^DECISION:priority$" | head -1 | cut -d: -f1)"
    [ -n "$stop_line" ]
    [ -n "$prio_line" ]
    [ "$stop_line" -lt "$prio_line" ]
}

# ---------------------------------------------------------------------------
# autospec:promote → DECISION:promote (trusted actor) or DECISION:promote-refused
#
# Trust is resolved against the real repo's .autospec/autospec.yml
# (safety.issue_intent_gate.trusted_actors: berlinguyinca), same convention
# as tests/autonomous/test_provenance_resolution.bats: "berlinguyinca" is
# the trusted login, any other login is untrusted. Promote verifies BOTH the
# issue author AND the actor of the last `autospec:promote` labeled timeline
# event (the label is the command authority), so the stub answers `gh issue
# list --label autospec:promote` AND `gh api .../timeline`.
# ---------------------------------------------------------------------------

# Helper: gh stub for promote — routes `issue list` with the promote label to
# the label fixture and `api .../timeline` to the timeline fixture; all other
# invocations return "[]". Pass "FAIL" as the timeline fixture to make the
# api call exit 1 (timeline unavailable → must fail closed).
make_gh_promote_stub() {
    local label_fixture="$1" timeline_fixture="$2"
    cat > "$TMP/bin/gh" <<EOF
#!/usr/bin/env bash
mode=""
found_label=""
for a in "\$@"; do
    case "\$a" in
        api) mode="api" ;;
        autospec:promote) found_label="1" ;;
    esac
done
if [ "\$mode" = "api" ]; then
    if [ "$timeline_fixture" = "FAIL" ]; then
        exit 1
    fi
    cat "$timeline_fixture"
    exit 0
fi
if [ -n "\$found_label" ]; then
    cat "$label_fixture"
else
    echo "[]"
fi
EOF
    chmod +x "$TMP/bin/gh"
}

@test "autospec:promote by trusted author + trusted label actor → DECISION:promote + PROMOTE_ISSUE" {
    local fixture="$TMP/promote_trusted.json"
    local timeline="$TMP/promote_timeline.json"
    printf '[{"number":201,"title":"promote the roll-up","body":"ship it","author":{"login":"berlinguyinca"}}]' > "$fixture"
    printf '[{"event":"labeled","label":{"name":"autospec:promote"},"actor":{"login":"berlinguyinca"}}]' > "$timeline"
    make_gh_promote_stub "$fixture" "$timeline"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:promote$"
    printf '%s\n' "$output" | grep -q "^PROMOTE_ISSUE:201$"
}

@test "autospec:promote by untrusted author is refused (no DECISION:promote)" {
    local fixture="$TMP/promote_untrusted.json"
    local timeline="$TMP/promote_timeline.json"
    printf '[{"number":202,"title":"promote the roll-up","body":"ship it","author":{"login":"some-random-user"}}]' > "$fixture"
    printf '[{"event":"labeled","label":{"name":"autospec:promote"},"actor":{"login":"berlinguyinca"}}]' > "$timeline"
    make_gh_promote_stub "$fixture" "$timeline"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:promote-refused$"
    printf '%s\n' "$output" | grep -q "^PROMOTE_ISSUE:202$"
    # Must NEVER emit the trusted decision token for an untrusted actor.
    ! printf '%s\n' "$output" | grep -q "^DECISION:promote$"
}

@test "autospec:promote label applied by untrusted actor is refused (label spoofing)" {
    local fixture="$TMP/promote_spoof.json"
    local timeline="$TMP/promote_spoof_timeline.json"
    # Trusted author, but the LAST autospec:promote labeled event was actioned
    # by an untrusted triager — the label is the command authority.
    printf '[{"number":204,"title":"old trusted issue","body":"ship it","author":{"login":"berlinguyinca"}}]' > "$fixture"
    printf '[{"event":"labeled","label":{"name":"autospec:promote"},"actor":{"login":"drive-by-triager"}}]' > "$timeline"
    make_gh_promote_stub "$fixture" "$timeline"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:promote-refused$"
    ! printf '%s\n' "$output" | grep -q "^DECISION:promote$"
}

@test "autospec:promote with unavailable timeline is refused (fail closed)" {
    local fixture="$TMP/promote_no_timeline.json"
    printf '[{"number":205,"title":"promote the roll-up","body":"ship it","author":{"login":"berlinguyinca"}}]' > "$fixture"
    make_gh_promote_stub "$fixture" "FAIL"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:promote-refused$"
    ! printf '%s\n' "$output" | grep -q "^DECISION:promote$"
}

@test "autospec:promote with missing author resolves untrusted (fail closed)" {
    local fixture="$TMP/promote_no_author.json"
    local timeline="$TMP/promote_timeline.json"
    printf '[{"number":203,"title":"promote the roll-up","body":"ship it"}]' > "$fixture"
    printf '[{"event":"labeled","label":{"name":"autospec:promote"},"actor":{"login":"berlinguyinca"}}]' > "$timeline"
    make_gh_promote_stub "$fixture" "$timeline"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:promote-refused$"
    ! printf '%s\n' "$output" | grep -q "^DECISION:promote$"
}

# ---------------------------------------------------------------------------
# autospec:discard → DECISION:discard + DISCARD_ISSUE
# ---------------------------------------------------------------------------

@test "autospec:discard label → DECISION:discard + DISCARD_ISSUE" {
    local fixture="$TMP/discard.json"
    printf '[{"number":301,"title":"discard the roll-up","body":"abandon it","author":{"login":"berlinguyinca"}}]' > "$fixture"
    make_gh_label_stub "autospec:discard" "$fixture"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q "^DECISION:discard$"
    printf '%s\n' "$output" | grep -q "^DISCARD_ISSUE:301$"
}

# ---------------------------------------------------------------------------
# gh failure is fail-open (no crash, no output)
# ---------------------------------------------------------------------------

@test "gh failure is fail-open: exits 0 with no decisions" {
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
    chmod +x "$TMP/bin/gh"

    run bash "$CONTROL_CHANNEL"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ---------------------------------------------------------------------------
# --help exits 0
# ---------------------------------------------------------------------------

@test "--help exits 0" {
    run bash "$CONTROL_CHANNEL" --help
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -qi "usage"
}

# ---------------------------------------------------------------------------
# Unknown argument exits 2
# ---------------------------------------------------------------------------

@test "unknown argument exits 2" {
    run bash "$CONTROL_CHANNEL" --unknown-flag
    [ "$status" -eq 2 ]
}
