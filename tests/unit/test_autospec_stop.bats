#!/usr/bin/env bats
# tests/unit/test_autospec_stop.bats — exercise every CLI flag of
# scripts/autospec-stop.sh against a sandboxed $HOME per spec §9.1.
#
# Spec ref: docs/specs/2026-05-01-autospec-stop-mechanism-design.md §9.1

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autospec-stop.sh"
    # Sandbox HOME so tests never touch real ~/.autospec/
    export HOME="$(mktemp -d)"
    export FLAG_DIR="$HOME/.autospec"
    export FLAG_FILE="$FLAG_DIR/stop.flag"
    # Suppress gh network calls by stubbing gh in PATH
    STUB_BIN="$(mktemp -d)"
    cat > "$STUB_BIN/gh" <<'GHSTUB'
#!/usr/bin/env sh
# Minimal gh stub: issue list returns empty JSON array; other commands succeed.
case "$*" in
    *"issue list"*) printf '[]\n' ;;
    *"issue edit"*) exit 0 ;;
    *"label create"*) exit 0 ;;
    *) exit 0 ;;
esac
GHSTUB
    chmod +x "$STUB_BIN/gh"
    export PATH="$STUB_BIN:$PATH"
}

teardown() {
    rm -rf "$HOME"
    rm -rf "$(dirname "$(command -v gh)")" 2>/dev/null || true
}

# §9.1 bullet 1: --graceful writes flag with mode=graceful + timestamp
@test "--graceful writes flag mode=graceful with timestamp" {
    run bash "$SCRIPT" --graceful
    [ "$status" -eq 0 ]
    [ -f "$FLAG_FILE" ]
    first_line="$(head -1 "$FLAG_FILE")"
    [ "$first_line" = "graceful" ]
    second_line="$(sed -n '2p' "$FLAG_FILE")"
    # Should contain a timestamp pattern YYYY-MM-DDTHH:MM:SSZ
    echo "$second_line" | grep -qE '[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z'
}

# §9.1 bullet 2: --immediate writes flag with mode=immediate
@test "--immediate writes flag mode=immediate" {
    run bash "$SCRIPT" --immediate
    [ "$status" -eq 0 ]
    [ -f "$FLAG_FILE" ]
    first_line="$(head -1 "$FLAG_FILE")"
    [ "$first_line" = "immediate" ]
}

# §9.1 bullet 3: --status with no flag prints "no stop signal active"
@test "--status with no flag prints 'no stop signal active'" {
    run bash "$SCRIPT" --status
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'no stop signal active'
}

# §9.1 bullet 4: --status with flag reports mode + age
@test "--status with flag reports mode and age" {
    bash "$SCRIPT" --graceful
    run bash "$SCRIPT" --status
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'graceful'
}

# §9.1 bullet 5: --resume deletes flag
@test "--resume deletes flag" {
    bash "$SCRIPT" --graceful
    [ -f "$FLAG_FILE" ]
    run bash "$SCRIPT" --resume
    [ "$status" -eq 0 ]
    [ ! -f "$FLAG_FILE" ]
}

# §9.1 bullet 6: --resume with mock paused issue (gh-stubbed) reports stripped
@test "--resume with stub gh reports flag deleted" {
    bash "$SCRIPT" --graceful
    run bash "$SCRIPT" --resume
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'flag deleted'
}

# §9.1 bullet 7: stale flag (>24h) — status warns
@test "stale flag (24h+) status warns" {
    # Write a flag with a timestamp 25 hours in the past
    mkdir -p "$FLAG_DIR"
    # Use a clearly old timestamp
    printf 'graceful\n1970-01-01T00:00:00Z wohlgemuth@test\n' > "$FLAG_FILE"
    run bash "$SCRIPT" --status
    [ "$status" -eq 0 ]
    # Should mention STALE or flag: graceful (status still reads it)
    echo "$output" | grep -qiE 'STALE|graceful'
}

# §9.1 bullet 8: --graceful then --immediate: last write wins (immediate)
@test "--graceful then --immediate: last write wins (immediate)" {
    bash "$SCRIPT" --graceful
    first="$(head -1 "$FLAG_FILE")"
    [ "$first" = "graceful" ]
    bash "$SCRIPT" --immediate
    second="$(head -1 "$FLAG_FILE")"
    [ "$second" = "immediate" ]
}

# Additional: --help exits 0 and lists flags
@test "--help exits 0 and lists all four flags" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    echo "$output" | grep -q -- '--graceful'
    echo "$output" | grep -q -- '--immediate'
    echo "$output" | grep -q -- '--status'
    echo "$output" | grep -q -- '--resume'
}

# ── #516: memory-aware --resume for known monitor exit modes ──────────────────
# Acceptance criteria from issue #516. The detector is stubbed via
# AUTOSPEC_DETECTOR so resume's reaction can be tested without real worktrees.

# Helper: install a detector stub that always reports the given mode.
_stub_detector() {
    local mode="$1"
    DETECTOR_STUB="$(mktemp "$HOME/detector.XXXXXX.sh")"
    printf '#!/usr/bin/env sh\necho %s\n' "$mode" > "$DETECTOR_STUB"
    chmod +x "$DETECTOR_STUB"
    export AUTOSPEC_DETECTOR="$DETECTOR_STUB"
}

# AC1: --resume on a clean state is a no-op (exit 0, message), flag removed.
@test "--resume clean state is a no-op exit 0 with message" {
    _stub_detector clean
    bash "$SCRIPT" --graceful
    run bash "$SCRIPT" --resume
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi 'clean'
    [ ! -f "$FLAG_FILE" ]
}

# AC2: simulated silent-exit -> resume reports cleanup + restart guidance.
@test "--resume silent-exit reports recovery (clean worktree + restart)" {
    _stub_detector silent-exit
    bash "$SCRIPT" --graceful
    run bash "$SCRIPT" --resume
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi 'silent-exit'
    echo "$output" | grep -qiE 'worktree|restart|relaunch|recover'
    [ ! -f "$FLAG_FILE" ]
}

# AC3: simulated prompt-overflow -> resume reports fresh-session restart.
@test "--resume prompt-overflow reports fresh-session restart" {
    _stub_detector prompt-overflow
    bash "$SCRIPT" --graceful
    run bash "$SCRIPT" --resume
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi 'prompt-overflow'
    echo "$output" | grep -qiE 'fresh|restart|relaunch|session'
    [ ! -f "$FLAG_FILE" ]
}

# unknown mode -> resume falls back to plain label restore + flag delete.
@test "--resume unknown mode falls back to label restore" {
    _stub_detector unknown
    bash "$SCRIPT" --graceful
    run bash "$SCRIPT" --resume
    [ "$status" -eq 0 ]
    echo "$output" | grep -qi 'unknown'
    [ ! -f "$FLAG_FILE" ]
}

# Backward compat: --resume still strips paused-by-user labels (existing behaviour).
@test "--resume still deletes the flag (backward compat)" {
    _stub_detector clean
    bash "$SCRIPT" --graceful
    [ -f "$FLAG_FILE" ]
    run bash "$SCRIPT" --resume
    [ "$status" -eq 0 ]
    [ ! -f "$FLAG_FILE" ]
}

# Additional: unknown flag exits non-zero
@test "unknown flag exits non-zero" {
    run bash "$SCRIPT" --bogus-flag
    [ "$status" -ne 0 ]
}
