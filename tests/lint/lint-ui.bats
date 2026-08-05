#!/usr/bin/env bats
# tests/lint/lint-ui.bats — deterministic design-token-drift linter (scripts/lint-ui.sh).

bats_require_minimum_version 1.5.0

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

@test "every rule compiles on the host awk (no interpreter panic on stderr)" {
    printf '.b { color: #3a7bd5; }\n' > "$TMP/a.css"
    run --separate-stderr bash "$L" "$TMP/a.css"
    [ -z "$stderr" ]
    printf '%s\n' "$output" | grep -q '^UI_RAW_HEX:'
}

@test "an awk failure reports an error instead of passing the file silently" {
    printf '.b { color: #3a7bd5; }\n' > "$TMP/a.css"
    AUTOSPEC_LINT_UI_AWK=false run bash "$L" "$TMP/a.css"
    [ "$status" -eq 2 ]
    printf '%s\n' "$output" | grep -q 'lint-ui:.*awk'
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

# --- motion / input rules (WCAG 2.2.2, 1.4.4, 2.3.3) ---

@test "UI_NO_REDUCED_MOTION: keyframe animation with no reduced-motion guard is flagged" {
    printf '@keyframes slide { from { transform: translateY(20px); } }\n.a { animation: slide 300ms; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_NO_REDUCED_MOTION:'
}

@test "UI_NO_REDUCED_MOTION: reduced-motion guard in the same file clears the rule" {
    printf '@keyframes slide { from { transform: translateY(20px); } }\n@media (prefers-reduced-motion: reduce) { .a { animation: none; } }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_NO_REDUCED_MOTION'
}

@test "UI_NO_REDUCED_MOTION: colour-only transition is not motion and is not flagged" {
    printf '.a { transition: color 200ms ease; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_NO_REDUCED_MOTION'
}

@test "UI_NO_REDUCED_MOTION: unguarded transform transition is flagged" {
    printf '.a { transition: transform 200ms ease; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_NO_REDUCED_MOTION:'
}

@test "UI_NO_REDUCED_MOTION: reports the first motion declaration's line" {
    printf '.a { color: var(--c); }\n.b { animation-name: spin; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q 'UI_NO_REDUCED_MOTION:.*:2:'
}

@test "UI_INFINITE_ANIMATION: infinite iteration count is flagged" {
    printf '.a { animation-iteration-count: infinite; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_INFINITE_ANIMATION:.*:1:'
}

@test "UI_INFINITE_ANIMATION: infinite in the animation shorthand is flagged" {
    printf '.a { animation: spin 2s linear infinite; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_INFINITE_ANIMATION:.*:1:'
}

@test "UI_INFINITE_ANIMATION: finite animation is not flagged" {
    printf '.a { animation: spin 2s linear 3; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_INFINITE_ANIMATION'
}

@test "UI_FIXED_VIEWPORT: user-scalable=no is flagged" {
    printf '<meta name="viewport" content="width=device-width, user-scalable=no">\n' > "$TMP/a.html"
    run bash "$L" "$TMP/a.html"
    printf '%s\n' "$output" | grep -q '^UI_FIXED_VIEWPORT:.*:1:'
}

@test "UI_FIXED_VIEWPORT: maximum-scale=1 is flagged" {
    printf '<meta name="viewport" content="width=device-width, maximum-scale=1.0">\n' > "$TMP/a.html"
    run bash "$L" "$TMP/a.html"
    printf '%s\n' "$output" | grep -q '^UI_FIXED_VIEWPORT:.*:1:'
}

@test "UI_FIXED_VIEWPORT: a zoomable viewport meta is not flagged" {
    printf '<meta name="viewport" content="width=device-width, initial-scale=1">\n' > "$TMP/a.html"
    run bash "$L" "$TMP/a.html"
    ! printf '%s\n' "$output" | grep -q 'UI_FIXED_VIEWPORT'
}

@test "UI_HOVER_ONLY_AFFORDANCE: hover with no focus equivalent is flagged" {
    printf '.a:hover { text-decoration: underline; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_HOVER_ONLY_AFFORDANCE:.*:1:'
}

@test "UI_HOVER_ONLY_AFFORDANCE: a focus-visible equivalent clears the rule" {
    printf '.a:hover { text-decoration: underline; }\n.a:focus-visible { text-decoration: underline; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_HOVER_ONLY_AFFORDANCE'
}

@test "--directives covers every motion/input rule" {
    printf '@keyframes s { from { transform: none; } }\n.a { animation: s 1s infinite; }\n.a:hover { color: var(--c); }\n' > "$TMP/a.css"
    run bash "$L" --directives "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^Fix UI_NO_REDUCED_MOTION:'
    printf '%s\n' "$output" | grep -q '^Fix UI_INFINITE_ANIMATION:'
    printf '%s\n' "$output" | grep -q '^Fix UI_HOVER_ONLY_AFFORDANCE:'
    printf '<meta name="viewport" content="user-scalable=no">\n' > "$TMP/b.html"
    run bash "$L" --directives "$TMP/b.html"
    printf '%s\n' "$output" | grep -q '^Fix UI_FIXED_VIEWPORT:'
}
