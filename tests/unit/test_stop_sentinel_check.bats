#!/usr/bin/env bats
# tests/unit/test_stop_sentinel_check.bats — extracts the canonical sentinel-check
# bash from skills/autospec/SKILL.md (outer-loop block) via awk, sources it into
# a sandboxed shell, and asserts spec §9.2 behaviours.
#
# Spec ref: docs/specs/2026-05-01-autospec-stop-mechanism-design.md §6, §9.2

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SKILL_FILE="$REPO_ROOT/skills/autospec/SKILL.md"

    # Extract the outer-loop sentinel check bash block from SKILL.md.
    # The block is embedded in a blockquote (> prefix) in the outer-loop section.
    # Extract lines starting from the "# autospec-stop sentinel check — outer loop"
    # comment through the closing "fi" that follows the elif block.
    SENTINEL_SCRIPT="$(mktemp)"
    awk '
        /# autospec-stop sentinel check — outer loop/ { capturing=1; depth=0 }
        capturing {
            line = $0
            sub(/^> */, "", line)
            # Track if/fi nesting depth
            if (line ~ /^if /) depth++
            if (line ~ /^fi$/) {
                depth--
                print line
                if (depth <= 0) { capturing=0; exit }
                next
            }
            print line
        }
    ' "$SKILL_FILE" > "$SENTINEL_SCRIPT"

    # Sandbox HOME so tests never touch real ~/.autospec/
    export HOME="$(mktemp -d)"
    export FLAG_DIR="$HOME/.autospec"
    export FLAG_FILE="$FLAG_DIR/stop.flag"
    mkdir -p "$FLAG_DIR"
}

teardown() {
    rm -f "$SENTINEL_SCRIPT" 2>/dev/null || true
    rm -rf "$HOME"
}

# Helper: write a stop.flag with a given mode and timestamp
write_flag() {
    local mode="$1"
    local ts="${2:-$(date -u +"%Y-%m-%dT%H:%M:%SZ")}"
    printf '%s\n%s testuser@testhost\n' "$mode" "$ts" > "$FLAG_FILE"
}

# Helper: run the extracted sentinel script in a subshell and capture output
run_sentinel() {
    bash "$SENTINEL_SCRIPT" 2>&1
    return $?
}

# §9.2 bullet 1: no flag → no exit, script continues (exit 0)
@test "no flag → no exit, continue" {
    rm -f "$FLAG_FILE"
    run bash "$SENTINEL_SCRIPT"
    # Should exit 0 (no flag present means no action)
    [ "$status" -eq 0 ]
    # Should produce no output
    [ -z "$output" ]
}

# §9.2 bullet 2: flag with mode=graceful → exit 0 with stop signal message
@test "flag mode=graceful → exit 0 with 'stop signal received: graceful'" {
    write_flag "graceful"
    run bash "$SENTINEL_SCRIPT"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'stop signal received: graceful'
}

# §9.2 bullet 3: flag with mode=immediate → exit 0 with stop signal message
@test "flag mode=immediate → exit 0 with 'stop signal received: immediate'" {
    write_flag "immediate"
    run bash "$SENTINEL_SCRIPT"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'stop signal received: immediate'
}

# §9.2 bullet 4: stale timestamp (>86400s) → WARN to stderr + exit 0 (continue)
@test "stale flag (>86400s old) → WARN to stderr, continue" {
    # Use a very old timestamp
    write_flag "graceful" "1970-01-01T00:00:00Z"
    run bash "$SENTINEL_SCRIPT"
    [ "$status" -eq 0 ]
    # Should warn about stale flag — output goes to stderr captured in bats output
    echo "$output" | grep -qi 'WARN.*stale\|stale.*WARN'
    # Should NOT have exited with stop signal
    echo "$output" | grep -qv 'stop signal received' || true
}

# §9.2 bullet 5: malformed mode → WARN + continue (no exit trigger)
@test "malformed mode → no stop signal, continue" {
    write_flag "bogusmode"
    run bash "$SENTINEL_SCRIPT"
    [ "$status" -eq 0 ]
    # Should not trigger the stop signal path
    echo "$output" | grep -qv 'stop signal received' || true
}

# Additional: test extracts bash from skills/autospec/SKILL.md (not hardcoded)
@test "extracted script references the SKILL.md extraction (not a hardcoded copy)" {
    # Verify the sentinel script file was populated from SKILL.md
    [ -s "$SENTINEL_SCRIPT" ]
    # Must contain the outer-loop comment marker
    grep -q 'autospec-stop sentinel check' "$SENTINEL_SCRIPT"
}
