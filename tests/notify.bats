#!/usr/bin/env bats
# tests/notify.bats — TDD for skills/autospec-shared/scripts/notify.sh (issue #1338)
#
# External boundaries (osascript / notify-send) are mocked via PATH stubs
# so no real desktop notifications fire during tests.
#
# PATH strategy:
#   Stub tests (osascript/notify-send present): "${STUB_DIR}:${PATH}" — stub
#     overrides any real notifier; PATH unchanged so bash/env work normally.
#   Fallback tests (no notifier): "${STUB_DIR}:/bin" — excludes /usr/bin so
#     the real macOS osascript (/usr/bin/osascript) is not found; /bin/bash
#     used directly so bash itself doesn't need to be on PATH.
#   Stub shebangs use #!/bin/bash (absolute) not #!/usr/bin/env bash so the
#     stub scripts execute even when PATH is restricted to STUB_DIR only.

bats_require_minimum_version 1.5.0

NOTIFY_SCRIPT="${BATS_TEST_DIRNAME}/../skills/autospec-shared/scripts/notify.sh"

setup() {
    STUB_DIR="$(mktemp -d)"
    CALL_LOG="${STUB_DIR}/calls.log"
    touch "$CALL_LOG"
}

teardown() {
    rm -rf "$STUB_DIR"
}

# ---------------------------------------------------------------------------
# AUTOSPEC_NOTIFY=0 → silent no-op, exit 0
# ---------------------------------------------------------------------------

@test "AUTOSPEC_NOTIFY=0 is a silent no-op and exits 0" {
    run env AUTOSPEC_NOTIFY=0 PATH="${STUB_DIR}:${PATH}" \
        bash "$NOTIFY_SCRIPT" "Test Title" "Test body"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ---------------------------------------------------------------------------
# notifier present (mock osascript) → invoked once with title and body
# ---------------------------------------------------------------------------

@test "osascript mock is invoked once when present on PATH" {
    # Create fake osascript that records its arguments.
    # Use #!/bin/bash (absolute) so it runs even on restricted PATH.
    cat > "${STUB_DIR}/osascript" <<'STUB'
#!/bin/bash
printf 'osascript called: %s\n' "$*" >> "${CALL_LOG}"
exit 0
STUB
    chmod +x "${STUB_DIR}/osascript"

    run env AUTOSPEC_NOTIFY=1 CALL_LOG="$CALL_LOG" PATH="${STUB_DIR}:${PATH}" \
        bash "$NOTIFY_SCRIPT" "My Title" "My body"
    [ "$status" -eq 0 ]

    # Verify osascript was called exactly once
    call_count="$(grep -c "osascript called:" "$CALL_LOG")"
    [ "$call_count" -eq 1 ]

    # Verify title appears in the call
    grep -q "My Title" "$CALL_LOG"
}

@test "osascript stub receives the body text" {
    cat > "${STUB_DIR}/osascript" <<'STUB'
#!/bin/bash
printf 'osascript called: %s\n' "$*" >> "${CALL_LOG}"
exit 0
STUB
    chmod +x "${STUB_DIR}/osascript"

    run env AUTOSPEC_NOTIFY=1 CALL_LOG="$CALL_LOG" PATH="${STUB_DIR}:${PATH}" \
        bash "$NOTIFY_SCRIPT" "T" "Hello world body"
    [ "$status" -eq 0 ]
    grep -q "Hello world body" "$CALL_LOG"
}

# ---------------------------------------------------------------------------
# notifier absent → stdout log fallback, exit 0
# ---------------------------------------------------------------------------

@test "no notifier on PATH prints fallback log line and exits 0" {
    # STUB_DIR has no osascript or notify-send.
    # PATH excludes /usr/bin so real macOS osascript is not found.
    # Use /bin/bash directly so bash itself doesn't need to be on PATH.
    run env AUTOSPEC_NOTIFY=1 PATH="${STUB_DIR}:/bin" \
        /bin/bash "$NOTIFY_SCRIPT" "FallbackTitle" "FallbackBody"
    [ "$status" -eq 0 ]
    [[ "$output" == *"notify: FallbackTitle"* ]]
    [[ "$output" == *"FallbackBody"* ]]
}

@test "fallback log line contains title and body" {
    run env AUTOSPEC_NOTIFY=1 PATH="${STUB_DIR}:/bin" \
        /bin/bash "$NOTIFY_SCRIPT" "MyT" "MyB"
    [ "$status" -eq 0 ]
    # Output must contain both title and body separated by the em-dash marker
    [[ "$output" == *"notify: MyT"* ]]
    [[ "$output" == *"MyB"* ]]
}

# ---------------------------------------------------------------------------
# notify-send (Linux mock) → invoked when osascript absent
# ---------------------------------------------------------------------------

@test "notify-send mock invoked when osascript absent" {
    # PATH excludes /usr/bin so real osascript is not found.
    # STUB_DIR has notify-send but not osascript.
    # Stub shebang is #!/bin/bash (absolute) so it runs on restricted PATH.
    cat > "${STUB_DIR}/notify-send" <<'STUB'
#!/bin/bash
printf 'notify-send called: %s\n' "$*" >> "${CALL_LOG}"
exit 0
STUB
    chmod +x "${STUB_DIR}/notify-send"

    run env AUTOSPEC_NOTIFY=1 CALL_LOG="$CALL_LOG" PATH="${STUB_DIR}:/bin" \
        /bin/bash "$NOTIFY_SCRIPT" "NSTitle" "NSBody"
    [ "$status" -eq 0 ]
    grep -q "notify-send called:" "$CALL_LOG"
    grep -q "NSTitle" "$CALL_LOG"
}

# ---------------------------------------------------------------------------
# notifier failure must not propagate non-zero exit
# ---------------------------------------------------------------------------

@test "failing osascript stub does not cause notify.sh to exit non-zero" {
    cat > "${STUB_DIR}/osascript" <<'STUB'
#!/bin/bash
exit 1
STUB
    chmod +x "${STUB_DIR}/osascript"

    run env AUTOSPEC_NOTIFY=1 PATH="${STUB_DIR}:${PATH}" \
        bash "$NOTIFY_SCRIPT" "T" "B"
    [ "$status" -eq 0 ]
}
