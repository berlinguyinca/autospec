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
