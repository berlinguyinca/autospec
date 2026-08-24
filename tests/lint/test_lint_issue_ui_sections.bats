#!/usr/bin/env bats
# tests/lint/test_lint_issue_ui_sections.bats — coverage for the ui-feature contract in
# scripts/lint-issue.sh: UI_SECTIONS_INCOMPLETE across all five required sections, and
# the spec §L1a exclusion that keeps those sections out of the BODY_TOO_LONG word count.
# Split out of test_lint_issue_sections.bats, which had grown past the file-size limit.
# The fixture helper is duplicated rather than sourced so each suite runs on its own.
# Fixtures are written as real temp files (bash 3.2; no process substitution).

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LINT="$REPO_ROOT/scripts/lint-issue.sh"
    TMP="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP"
}

# A complete, well-formed body that passes every rule. Callers append/override.
write_good_body() {
    cat > "$1" <<'MD'
## Goal

Add a deterministic gate to `scripts/lint-issue.sh` for required sections.

## Files to read first

- scripts/lint-issue.sh
- tests/unit/test_lint_issue.bats

## Implementation outline

1. Add the section-presence checks.
2. Wire them into the main run.
3. Extend the bats coverage.

## Tests required

- bats tests/lint/test_lint_issue_sections.bats

## Dependencies

Depends on issue #152

## Files touched

- scripts/lint-issue.sh

## Acceptance criteria

- [ ] `bash scripts/lint-issue.sh body.md` exits 0.

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/lint-issue.sh body.md && echo OK
```
MD
}

@test "UI_SECTIONS_INCOMPLETE: ui-feature marker without the UI sections is flagged" {
    write_good_body "$TMP/b.md"
    printf '\n<!-- ui-feature -->\n' >> "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'UI_SECTIONS_INCOMPLETE'
    printf '%s\n' "$output" | grep -E 'UI_SECTIONS_INCOMPLETE' | grep -q 'Design reference'
}

@test "UI_SECTIONS_INCOMPLETE: one UI section present requires the other four" {
    write_good_body "$TMP/b.md"
    printf '\n## Interaction states\n\ndefault/hover/focus/loading/empty/error\n' >> "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -ne 0 ]
    out="$(printf '%s\n' "$output" | grep 'UI_SECTIONS_INCOMPLETE')"
    # Inspect only the missing-list portion (before the explanatory parenthetical,
    # which always names all five sections).
    missing="$(printf '%s' "$out" | sed 's/ (UI issues.*//')"
    printf '%s' "$missing" | grep -q 'Design reference'
    printf '%s' "$missing" | grep -q 'UX flows'
    printf '%s' "$missing" | grep -q 'Motion & feedback'
    printf '%s' "$missing" | grep -q 'Device & viewport'
    ! printf '%s' "$missing" | grep -q 'Interaction states'
}

@test "UI_SECTIONS_INCOMPLETE: missing only Motion & feedback is flagged" {
    write_good_body "$TMP/b.md"
    printf '\n<!-- ui-feature -->\n\n## Design reference\n\nDESIGN.md#buttons\n\n## Interaction states\n\ndefault/hover/focus\n\n## UX flows\n\nhappy: click -> submit\n\n## Device & viewport\n\nDevices: iPhone SE; reflow-320: no h-scroll\n' >> "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -ne 0 ]
    out="$(printf '%s\n' "$output" | grep 'UI_SECTIONS_INCOMPLETE')"
    missing="$(printf '%s' "$out" | sed 's/ (UI issues.*//')"
    printf '%s' "$missing" | grep -q 'Motion & feedback'
    ! printf '%s' "$missing" | grep -q 'Design reference'
    ! printf '%s' "$missing" | grep -q 'Interaction states'
    ! printf '%s' "$missing" | grep -q 'UX flows'
    ! printf '%s' "$missing" | grep -q 'Device & viewport'
}

@test "UI_SECTIONS_INCOMPLETE: missing only Device & viewport is flagged" {
    write_good_body "$TMP/b.md"
    printf '\n<!-- ui-feature -->\n\n## Design reference\n\nDESIGN.md#buttons\n\n## Interaction states\n\ndefault/hover/focus\n\n## UX flows\n\nhappy: click -> submit\n\n## Motion & feedback\n\nMotion: fade-in; reduced: opacity-only\n' >> "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -ne 0 ]
    out="$(printf '%s\n' "$output" | grep 'UI_SECTIONS_INCOMPLETE')"
    missing="$(printf '%s' "$out" | sed 's/ (UI issues.*//')"
    printf '%s' "$missing" | grep -q 'Device & viewport'
    ! printf '%s' "$missing" | grep -q 'Design reference'
    ! printf '%s' "$missing" | grep -q 'Interaction states'
    ! printf '%s' "$missing" | grep -q 'UX flows'
    ! printf '%s' "$missing" | grep -q 'Motion & feedback'
}

@test "UI_SECTIONS_INCOMPLETE: complete UI section set (all five) passes" {
    write_good_body "$TMP/b.md"
    printf '\n<!-- ui-feature -->\n\n## Design reference\n\nDESIGN.md#buttons\n\n## Interaction states\n\ndefault/hover/focus/loading/empty/error/disabled\n\n## UX flows\n\nhappy: click -> submit; failure: 500 -> toast; edge: empty list\n\n## Motion & feedback\n\nMotion: fade-in + 40ms stagger; reduced: opacity-only\n\n## Device & viewport\n\nDevices: iPhone SE, Pixel 7, 1280x800 laptop; reflow-320: no h-scroll; zoom-200%%: no clipped text\n' >> "$TMP/b.md"
    # 2>&1 (distinct from the neighboring test) so this assertion line is its
    # own diff hunk rather than reusing unmodified context.
    run bash "$LINT" "$TMP/b.md" 2>&1
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'UI_SECTIONS_INCOMPLETE'
}

@test "UI_SECTIONS_INCOMPLETE: non-UI issue (no marker, no UI sections) is not flagged" {
    write_good_body "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'UI_SECTIONS_INCOMPLETE'
}

# ── word-cap exclusion for ui-feature sections (spec §L1a) ───────────────────

@test "BODY_TOO_LONG: ui-feature body with ~400-word prose plus all five sections passes" {
    {
        echo "## Goal"
        echo ""
        echo "Add a deterministic gate to \`scripts/lint-issue.sh\` for required sections."
        echo ""
        echo "## Files to read first"
        echo ""
        echo "- scripts/lint-issue.sh"
        echo ""
        echo "## Implementation outline"
        echo ""
        echo "1. Add the section-presence checks."
        echo ""
        echo "## Tests required"
        echo ""
        echo "- bats tests/lint/test_lint_issue_sections.bats"
        echo ""
        echo "## Dependencies"
        echo ""
        echo "none"
        echo ""
        echo "## Files touched"
        echo ""
        echo "- scripts/lint-issue.sh"
        echo ""
        echo "## Acceptance criteria"
        echo ""
        echo "- [ ] \`bash scripts/lint-issue.sh body.md\` exits 0."
        echo ""
        echo "## Notes"
        echo ""
        # 320 filler words + this body's own ~72 structural words puts the
        # non-UI prose just under the ≤400-word cap (~392); the five UI
        # sections below add ~64 more words that must NOT count toward it.
        for i in $(seq 1 320); do printf 'word '; done
        echo ""
        echo ""
        echo "## Verification"
        echo ""
        echo "### Primary smoke test (inner loop)"
        echo ""
        echo '```bash'
        echo "bash scripts/lint-issue.sh body.md && echo OK"
        echo '```'
        echo ""
        echo "<!-- ui-feature -->"
        echo ""
        echo "## Design reference"
        echo ""
        echo "DESIGN.md#buttons tokens spacing color type scale variant state"
        echo ""
        echo "## Interaction states"
        echo ""
        echo "default hover focus loading empty error disabled breakpoints changed"
        echo ""
        echo "## UX flows"
        echo ""
        echo "happy click submit failure toast edge empty list retry cancel"
        echo ""
        echo "## Motion & feedback"
        echo ""
        echo "Motion: fade-in plus stagger timing curve; reduced: opacity-only fallback"
        echo ""
        echo "## Device & viewport"
        echo ""
        echo "Devices: iPhone SE Pixel 7 laptop desktop; reflow-320 zoom-200 no clip"
    } > "$TMP/b.md"
    run bash -c "bash '$LINT' '$TMP/b.md' 2>&1"
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'BODY_TOO_LONG'
}

@test "BODY_TOO_LONG: UI headings with trailing whitespace are still excluded" {
    # check_ui_sections accepts a heading with trailing whitespace when deciding the
    # section is present, so strip_ui_sections has to accept it too. If the two
    # disagree, the section counts against the very cap it is exempt from — and
    # markdown's hard line break is two trailing spaces, so this really occurs.
    # Sized far from the boundary on purpose: the UI bodies alone are ~500 words while
    # the non-UI prose is ~30, so the outcome cannot hinge on counting arithmetic.
    # Every required section is present so the linter runs to completion — an
    # incomplete body makes this pass for the wrong reason, on empty output.
    local filler
    filler="$(for i in $(seq 1 100); do printf 'filler '; done)"
    {
        echo "## Goal"
        echo ""
        echo "Add a deterministic gate to \`scripts/lint-issue.sh\` for required sections."
        echo ""
        echo "## Files to read first"
        echo ""
        echo "- scripts/lint-issue.sh"
        echo ""
        echo "## Implementation outline"
        echo ""
        echo "1. Add the section-presence checks."
        echo ""
        echo "## Tests required"
        echo ""
        echo "- bats tests/lint/test_lint_issue_sections.bats"
        echo ""
        echo "## Dependencies"
        echo ""
        echo "none"
        echo ""
        echo "## Verification"
        echo ""
        echo "### Primary smoke test (inner loop)"
        echo ""
        echo '```bash'
        echo "bash scripts/lint-issue.sh body.md && echo OK"
        echo '```'
        echo ""
        echo "## Acceptance criteria"
        echo ""
        echo "- [ ] \`bash scripts/lint-issue.sh body.md\` exits 0."
        echo ""
        # Each heading carries two trailing spaces: markdown's hard line break, and the
        # case at issue. Written out rather than looped so the fixture stays flat.
        printf '## Design reference  \n\n%s\n\n' "$filler"
        printf '## Interaction states  \n\n%s\n\n' "$filler"
        printf '## UX flows  \n\n%s\n\n' "$filler"
        printf '## Motion & feedback  \n\n%s\n\n' "$filler"
        printf '## Device & viewport  \n\n%s\n\n' "$filler"
    } > "$TMP/b.md"
    run bash -c "bash '$LINT' '$TMP/b.md' 2>&1"
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'BODY_TOO_LONG'
    # The same headings must still register as present, or this would pass because the
    # sections went invisible to both checks rather than because they are exempt.
    ! printf '%s\n' "$output" | grep -q 'UI_SECTIONS_INCOMPLETE'
}

@test "BODY_TOO_LONG: non-UI prose over 400 words still trips the cap despite all five sections" {
    {
        echo "## Goal"
        echo ""
        echo "Add a deterministic gate to \`scripts/lint-issue.sh\` for required sections."
        echo ""
        echo "## Files to read first"
        echo ""
        echo "- scripts/lint-issue.sh"
        echo ""
        echo "## Implementation outline"
        echo ""
        echo "1. Add the section-presence checks."
        echo ""
        echo "## Tests required"
        echo ""
        echo "- bats tests/lint/test_lint_issue_sections.bats"
        echo ""
        echo "## Dependencies"
        echo ""
        echo "none"
        echo ""
        echo "## Files touched"
        echo ""
        echo "- scripts/lint-issue.sh"
        echo ""
        echo "## Acceptance criteria"
        echo ""
        echo "- [ ] \`bash scripts/lint-issue.sh body.md\` exits 0."
        echo ""
        echo "## Notes"
        echo ""
        # 450 words of non-UI filler — over cap on its own.
        for i in $(seq 1 450); do printf 'word '; done
        echo ""
        echo ""
        echo "## Verification"
        echo ""
        echo "### Primary smoke test (inner loop)"
        echo ""
        echo '```bash'
        echo "bash scripts/lint-issue.sh body.md && echo OK"
        echo '```'
        echo ""
        echo "<!-- ui-feature -->"
        echo ""
        echo "## Design reference"
        echo ""
        echo "DESIGN.md#buttons"
        echo ""
        echo "## Interaction states"
        echo ""
        echo "default/hover/focus"
        echo ""
        echo "## UX flows"
        echo ""
        echo "happy: click -> submit"
        echo ""
        echo "## Motion & feedback"
        echo ""
        echo "Motion: fade-in; reduced: opacity-only"
        echo ""
        echo "## Device & viewport"
        echo ""
        echo "Devices: iPhone SE; reflow-320: no h-scroll"
    } > "$TMP/b.md"
    run bash -c "bash '$LINT' '$TMP/b.md' 2>&1"
    [ "$status" -ge 1 ]
    printf '%s\n' "$output" | grep -q 'BODY_TOO_LONG'
    ! printf '%s\n' "$output" | grep -q 'UI_SECTIONS_INCOMPLETE'
}

@test "BODY_TOO_LONG: a generated block trailing the UI sections is still exempt" {
    # Regression: strip_ui_sections skips every line after a UI heading until the
    # next '## ' line. A generated block opens with '<!-- autospec-*:begin -->',
    # which is not one, so stripping UI sections FIRST deletes the opening marker
    # and strip_generated_metadata then declines to strip anything -- charging the
    # whole block to the authored count. strip_non_authored_sections therefore
    # removes generated metadata before UI sections; this pins that ordering.
    write_good_body "$TMP/b.md"
    {
        printf '\n<!-- ui-feature -->\n\n'
        echo "## Design reference"
        echo ""
        echo "DESIGN.md#buttons"
        echo ""
        echo "## Interaction states"
        echo ""
        echo "default/hover/focus"
        echo ""
        echo "## UX flows"
        echo ""
        echo "happy: click -> submit"
        echo ""
        echo "## Motion & feedback"
        echo ""
        echo "Motion: fade-in; reduced: opacity-only"
        echo ""
        echo "## Device & viewport"
        echo ""
        echo "Devices: iPhone SE; reflow-320: no h-scroll"
        echo ""
        echo "<!-- autospec-classify:begin -->"
        echo "## Model fit"
        echo ""
        for _ in $(seq 1 40); do
            echo "generated generated generated generated generated generated generated generated"
        done
        echo "<!-- autospec-classify:end -->"
    } > "$TMP/b.md.new"
    cat "$TMP/b.md" "$TMP/b.md.new" > "$TMP/c.md"
    run bash -c "bash '$LINT' '$TMP/c.md' 2>&1"
    ! printf '%s\n' "$output" | grep -q 'BODY_TOO_LONG'
}

@test "BODY_TOO_LONG: authored prose after a generated block is still counted" {
    # The mirror of the test above. strip_ui_sections must resume counting once a
    # generated block ends the UI section; otherwise the skip runs past the block
    # and silently exempts real authored prose that follows it.
    write_good_body "$TMP/b.md"
    {
        cat "$TMP/b.md"
        printf '\n<!-- ui-feature -->\n\n'
        echo "## Design reference"
        echo ""
        echo "DESIGN.md#buttons"
        echo ""
        echo "## Interaction states"
        echo ""
        echo "default/hover/focus"
        echo ""
        echo "## UX flows"
        echo ""
        echo "happy: click -> submit"
        echo ""
        echo "## Motion & feedback"
        echo ""
        echo "Motion: fade-in; reduced: opacity-only"
        echo ""
        echo "## Device & viewport"
        echo ""
        echo "Devices: iPhone SE; reflow-320: no h-scroll"
        echo ""
        echo "<!-- autospec-classify:begin -->"
        echo "## Model fit"
        echo ""
        echo "- ctx"
        echo "<!-- autospec-classify:end -->"
        echo ""
        for _ in $(seq 1 60); do
            echo "authored authored authored authored authored authored authored authored"
        done
    } > "$TMP/c.md"
    run bash -c "bash '$LINT' '$TMP/c.md' 2>&1"
    [ "$status" -ge 1 ]
    printf '%s\n' "$output" | grep -q 'BODY_TOO_LONG'
}
