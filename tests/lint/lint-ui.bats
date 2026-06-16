#!/usr/bin/env bats
# tests/lint/lint-ui.bats — deterministic design-token-drift linter (scripts/lint-ui.sh).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    L="$REPO_ROOT/scripts/lint-ui.sh"
    TMP="$(mktemp -d)"
}
teardown() { rm -rf "$TMP"; }

@test "--help prints a Usage line" {
    run bash "$L" --help
    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q 'Usage:'
}

@test "UI_RAW_HEX: raw hex color value is flagged" {
    printf '.b { color: #3a7bd5; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_RAW_HEX:'
    [ "$status" -ge 1 ]
}

@test "UI_RAW_HEX: CSS variable / token usage is NOT flagged" {
    printf '.b { color: var(--primary); }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_RAW_HEX'
}

@test "UI_RAW_HEX: quoted hex in a JS string is flagged" {
    printf 'const c = "#ff0066";\n' > "$TMP/a.js"
    run bash "$L" "$TMP/a.js"
    printf '%s\n' "$output" | grep -q '^UI_RAW_HEX:'
}

@test "UI_OFF_GRID_SPACING: off-grid padding flagged, on-grid not" {
    printf '.a { padding: 13px; }\n.b { padding: 8px 16px; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q 'UI_OFF_GRID_SPACING:.*:1:'
    ! printf '%s\n' "$output" | grep -q 'UI_OFF_GRID_SPACING:.*:2:'
}

@test "UI_OFF_GRID_SPACING: 2px hairline is not flagged" {
    printf '.a { padding: 2px; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_OFF_GRID_SPACING'
}

@test "UI_AD_HOC_ZINDEX: off-scale z-index flagged, scale value not" {
    printf '.a { z-index: 9999; }\n.b { z-index: 100; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q 'UI_AD_HOC_ZINDEX:.*:1:'
    ! printf '%s\n' "$output" | grep -q 'UI_AD_HOC_ZINDEX:.*:2:'
}

@test "UI_BANNED_FONT: banned font flagged, token font not" {
    printf '.a { font-family: Inter, sans-serif; }\n.b { font-family: var(--font-display); }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q 'UI_BANNED_FONT:.*:1:'
    ! printf '%s\n' "$output" | grep -q 'UI_BANNED_FONT:.*:2:'
}

@test "clean UI file -> exit 0, no findings" {
    printf '.a { color: var(--c); padding: 8px; z-index: 20; font-family: var(--f); }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "token/theme source files are skipped" {
    printf '.a { color: #3a7bd5; z-index: 9999; }\n' > "$TMP/theme.css"
    run bash "$L" "$TMP/theme.css"
    [ "$status" -eq 0 ]
}

@test "exit code equals finding count" {
    printf '.a { color: #abc; padding: 13px; z-index: 7; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    [ "$status" -eq 3 ]
}

@test "--directives reformats findings as Fix lines" {
    printf '.a { color: #abc; }\n' > "$TMP/a.css"
    run bash "$L" --directives "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^Fix UI_RAW_HEX:'
}
