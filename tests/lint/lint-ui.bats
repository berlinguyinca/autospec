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
    # 99, not 2: the finding count occupies the low codes, so 2 meant both "two
    # findings" and "the linter is broken".
    [ "$status" -eq 99 ]
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

@test "UI_RAW_HEX: a custom-property definition is the token source, not a violation" {
    # Measured on berlinguyinca/autospec-gui: 11 of its 21 raw-hex findings were the :root
    # block that *defines* its palette. Telling an author to replace those with a token is
    # circular — they are the token. The pilot corpus never caught it because its palette
    # lives in styles/tokens.css, whose name matches the filename skip list, so the fixture
    # quietly encoded the gate's own assumption.
    printf ':root {\n  --accent: #0f766e;\n  --bad: #dc2626;\n}\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_RAW_HEX'
}

@test "UI_RAW_HEX: a real usage beside a definition on one line is still flagged" {
    printf '.b { --accent: #0f766e; color: #3a7bd5; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_RAW_HEX:'
}

@test "UI_RAW_HEX: a hardcoded var() fallback is still a raw hex" {
    printf '.b { color: var(--primary, #3a7bd5); }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_RAW_HEX:'
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

# ── comments are not code ─────────────────────────────────────────────────────
# Found by running the gate over berlinguyinca/autospec-ui-pilot. The rules matched
# their own subject matter inside comments, in both directions: a comment naming a
# banned viewport directive was reported, and a comment naming the reduced-motion
# media feature silenced a real finding.

@test "a comment naming prefers-reduced-motion does not silence the motion rule" {
    # The worst of the pair: the note about the missing guard removed the finding about
    # the missing guard, and the file read as compliant from then on.
    printf '/* TODO: add prefers-reduced-motion here */\n@keyframes s { from { transform: translateY(4px); } }\n.a { animation: s 200ms ease-out; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_NO_REDUCED_MOTION:'
}

@test "a real prefers-reduced-motion guard still clears the motion rule" {
    printf '@keyframes s { from { transform: translateY(4px); } }\n.a { animation: s 200ms ease-out; }\n@media (prefers-reduced-motion: reduce) { .a { animation-name: none; } }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_NO_REDUCED_MOTION'
}

@test "a multi-line CSS comment does not silence the motion rule" {
    printf '/*\n * Motion here has no fallback for prefers-reduced-motion yet.\n */\n@keyframes s { from { transform: translateY(4px); } }\n.a { animation: s 200ms ease-out; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_NO_REDUCED_MOTION:'
}

@test "an HTML comment naming a banned viewport directive is not flagged" {
    printf '<!-- Never use user-scalable=no or maximum-scale=1 -->\n<meta name="viewport" content="width=device-width, initial-scale=1">\n' > "$TMP/a.html"
    run bash "$L" "$TMP/a.html"
    ! printf '%s\n' "$output" | grep -q 'UI_FIXED_VIEWPORT'
}

@test "prose naming a banned viewport directive is not flagged" {
    # A style guide documenting the anti-pattern is not committing it.
    printf '<meta name="viewport" content="width=device-width, initial-scale=1">\n<p>Never set user-scalable=no: it blocks zoom.</p>\n' > "$TMP/a.html"
    run bash "$L" "$TMP/a.html"
    ! printf '%s\n' "$output" | grep -q 'UI_FIXED_VIEWPORT'
}

@test "a zoom-blocking viewport split across lines is still flagged" {
    # The directive rarely shares a line with name="viewport" once a formatter has been
    # through the file, so the rule has to carry the meta tag's context across lines.
    printf '<meta\n  name="viewport"\n  content="width=device-width, maximum-scale=1, user-scalable=no"\n/>\n' > "$TMP/a.html"
    run bash "$L" "$TMP/a.html"
    printf '%s\n' "$output" | grep -q '^UI_FIXED_VIEWPORT:'
}

@test "a banned directive outside any meta tag is not flagged" {
    printf '<script>\n  const help = "do not pass user-scalable=no";\n</script>\n' > "$TMP/a.html"
    run bash "$L" "$TMP/a.html"
    ! printf '%s\n' "$output" | grep -q 'UI_FIXED_VIEWPORT'
}

# ── raw hex beyond the first token ────────────────────────────────────────────

@test "UI_RAW_HEX: hex inside a shorthand value is flagged" {
    printf '.a { border: 1px solid #cccccc; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    printf '%s\n' "$output" | grep -q '^UI_RAW_HEX:'
}

@test "UI_RAW_HEX: hex inside a gradient and a var() fallback is flagged" {
    printf '.a { background: linear-gradient(to bottom, #ffffff, #eeeeee); }\n.b { color: var(--x, #333333); }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    [ "$(printf '%s\n' "$output" | grep -c '^UI_RAW_HEX:')" -eq 2 ]
}

@test "UI_RAW_HEX: the four- and eight-digit alpha forms are flagged" {
    # Matching exactly three or six digits let #RRGGBBAA through, which is the form a
    # shadow or overlay colour usually takes.
    printf '.a { box-shadow: 0 1px 2px #00000022; }\n.b { color: #abcd; }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    [ "$(printf '%s\n' "$output" | grep -c '^UI_RAW_HEX:')" -eq 2 ]
}

@test "UI_RAW_HEX: hex inside a comment is not flagged" {
    printf '/* the old value was #3a7bd5 */\n.a { color: var(--brand); }\n' > "$TMP/a.css"
    run bash "$L" "$TMP/a.css"
    ! printf '%s\n' "$output" | grep -q 'UI_RAW_HEX'
}

# ── exit codes ────────────────────────────────────────────────────────────────

@test "a broken interpreter is distinguishable from a two-finding file" {
    # Both used to exit 2, so a caller could not tell a dead gate from an ordinary
    # result — which is most of what the fail-loud check was added to prevent.
    printf '.a { color: #abc; padding: 13px; }\n' > "$TMP/two.css"
    run bash "$L" "$TMP/two.css"
    [ "$status" -eq 2 ]

    AUTOSPEC_LINT_UI_AWK=false run bash "$L" "$TMP/two.css"
    [ "$status" -eq 99 ]
    printf '%s\n' "$output" | grep -q 'lint-ui:.*awk'
}

# ── global reduced-motion reset across files ──────────────────────────────────
# UI_NO_REDUCED_MOTION is decided per file, so a project keeping its reset in one global
# stylesheet and animating in components — the ordinary way to organise CSS — saw a
# finding on every component. A reset that targets the universal selector genuinely
# guards every element on the page, so when one is present in the same invocation the
# rule is satisfied.

@test "a global reset in the same invocation clears the motion rule" {
    printf '@media (prefers-reduced-motion: reduce) {\n  *,\n  *::before {\n    animation-duration: 0.01ms !important;\n  }\n}\n' > "$TMP/reset.css"
    printf '@keyframes s { from { transform: translateY(4px); } }\n.a { animation: s 200ms ease-out; }\n' > "$TMP/card.css"
    run bash "$L" "$TMP/reset.css" "$TMP/card.css"
    ! printf '%s\n' "$output" | grep -q 'UI_NO_REDUCED_MOTION'
}

@test "the same component alone is still flagged" {
    # Linting one file cannot see a reset that lives in another, so the pre-commit path
    # still reports it. The rule is satisfied by evidence, not by assumption.
    printf '@keyframes s { from { transform: translateY(4px); } }\n.a { animation: s 200ms ease-out; }\n' > "$TMP/card.css"
    run bash "$L" "$TMP/card.css"
    printf '%s\n' "$output" | grep -q '^UI_NO_REDUCED_MOTION:'
}

@test "a scoped reduced-motion block does not count as a global reset" {
    # Guarding one component does not guard the others; only a universal selector does.
    printf '@media (prefers-reduced-motion: reduce) {\n  .panel {\n    animation-name: none;\n  }\n}\n' > "$TMP/reset.css"
    printf '@keyframes s { from { transform: translateY(4px); } }\n.a { animation: s 200ms ease-out; }\n' > "$TMP/card.css"
    run bash "$L" "$TMP/reset.css" "$TMP/card.css"
    printf '%s\n' "$output" | grep -q '^UI_NO_REDUCED_MOTION:'
}

@test "a universal selector unrelated to reduced motion does not count" {
    printf '*,\n*::before {\n  box-sizing: border-box;\n}\n' > "$TMP/reset.css"
    printf '@keyframes s { from { transform: translateY(4px); } }\n.a { animation: s 200ms ease-out; }\n' > "$TMP/card.css"
    run bash "$L" "$TMP/reset.css" "$TMP/card.css"
    printf '%s\n' "$output" | grep -q '^UI_NO_REDUCED_MOTION:'
}

@test "a block comment is not mistaken for a global reset" {
    # A comment continuation line begins with '*', which is the shape the global-reset
    # scan looks for. Reading comments would let a note about a missing guard pose as a
    # reset — the silencing bug rebuilt in a second place, which is what happened when
    # this scan was first written.
    printf '/*\n * No prefers-reduced-motion fallback here yet.\n */\n.a { color: var(--c); }\n' > "$TMP/notes.css"
    printf '@keyframes s { from { transform: translateY(4px); } }\n.b { animation: s 200ms ease-out; }\n' > "$TMP/card.css"
    run bash "$L" "$TMP/notes.css" "$TMP/card.css"
    printf '%s\n' "$output" | grep -q '^UI_NO_REDUCED_MOTION:'
}

@test "a global reset does not suppress the other motion rules" {
    printf '@media (prefers-reduced-motion: reduce) {\n  * {\n    animation-duration: 0.01ms !important;\n  }\n}\n' > "$TMP/reset.css"
    printf '.a { animation: spin 2s linear infinite; }\n' > "$TMP/spin.css"
    run bash "$L" "$TMP/reset.css" "$TMP/spin.css"
    printf '%s\n' "$output" | grep -q '^UI_INFINITE_ANIMATION:'
}
