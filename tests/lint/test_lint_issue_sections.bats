#!/usr/bin/env bats
# tests/lint/test_lint_issue_sections.bats — coverage for the implementer-load-bearing
# section + sizing RULE_IDs added to scripts/lint-issue.sh (autospec #420/#421):
#   MISSING_SECTION_FILES_TO_READ, MISSING_SECTION_IMPL_OUTLINE,
#   MISSING_SECTION_TESTS, DEPS_MALFORMED, TOO_MANY_FILES, BODY_TOO_LONG,
#   OUTLINE_TOO_LONG, plus the relaxed GOAL_NOT_ONE_SENTENCE rule.
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

# ── baseline: the complete body passes ────────────────────────────────────────

@test "sections: complete well-formed body exits 0 with no findings" {
    write_good_body "$TMP/good.md"
    run bash "$LINT" "$TMP/good.md"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

# ── MISSING_SECTION_* ─────────────────────────────────────────────────────────

@test "sections: missing Files to read first triggers MISSING_SECTION_FILES_TO_READ" {
    write_good_body "$TMP/b.md"
    # Drop the heading line.
    grep -v '^## Files to read first$' "$TMP/b.md" > "$TMP/b2.md"
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "MISSING_SECTION_FILES_TO_READ"
}

@test "sections: missing Implementation outline triggers MISSING_SECTION_IMPL_OUTLINE" {
    write_good_body "$TMP/b.md"
    grep -v '^## Implementation outline$' "$TMP/b.md" > "$TMP/b2.md"
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "MISSING_SECTION_IMPL_OUTLINE"
}

@test "sections: missing Tests required triggers MISSING_SECTION_TESTS" {
    write_good_body "$TMP/b.md"
    grep -v '^## Tests required$' "$TMP/b.md" > "$TMP/b2.md"
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "MISSING_SECTION_TESTS"
}

# ── DEPS_MALFORMED ────────────────────────────────────────────────────────────

@test "deps: bad dependency line triggers DEPS_MALFORMED" {
    write_good_body "$TMP/b.md"
    # Replace the valid 'Depends on issue #152' with a bad form.
    sed 's/^Depends on issue #152$/blocked by #5/' "$TMP/b.md" > "$TMP/b2.md"
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "DEPS_MALFORMED"
}

@test "deps: 'Depends on issue #N' is accepted (no DEPS_MALFORMED)" {
    write_good_body "$TMP/b.md"
    run bash -c "bash '$LINT' '$TMP/b.md' 2>&1"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "DEPS_MALFORMED"
}

@test "deps: 'none' is accepted (no DEPS_MALFORMED)" {
    write_good_body "$TMP/b.md"
    sed 's/^Depends on issue #152$/none/' "$TMP/b.md" > "$TMP/b2.md"
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "DEPS_MALFORMED"
}

# ── TOO_MANY_FILES ────────────────────────────────────────────────────────────

@test "files-touched: 4 paths triggers TOO_MANY_FILES" {
    write_good_body "$TMP/b.md"
    # Replace the single Files touched bullet with four bullets.
    cat > "$TMP/files.md" <<'MD'
## Files touched

- scripts/a.sh
- scripts/b.sh
- scripts/c.sh
- scripts/d.sh
MD
    # Reassemble: drop existing Files touched section, then we will just craft body.
    cat > "$TMP/b2.md" <<'MD'
## Goal

Touch four files in `scripts/`.

## Files to read first

- scripts/a.sh

## Implementation outline

1. Edit the files.

## Tests required

- bats tests/x.bats

## Files touched

- scripts/a.sh
- scripts/b.sh
- scripts/c.sh
- scripts/d.sh

## Acceptance criteria

- [ ] `bash scripts/a.sh` exits 0.

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/a.sh && echo OK
```
MD
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "TOO_MANY_FILES"
}

@test "files-touched: 3 paths is accepted (no TOO_MANY_FILES)" {
    cat > "$TMP/b.md" <<'MD'
## Goal

Touch three files in `scripts/`.

## Files to read first

- scripts/a.sh

## Implementation outline

1. Edit the files.

## Tests required

- bats tests/x.bats

## Files touched

- scripts/a.sh
- scripts/b.sh
- scripts/c.sh

## Acceptance criteria

- [ ] `bash scripts/a.sh` exits 0.

## Verification

### Primary smoke test (inner loop)

```bash
bash scripts/a.sh && echo OK
```
MD
    run bash -c "bash '$LINT' '$TMP/b.md' 2>&1"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "TOO_MANY_FILES"
}

# ── BODY_TOO_LONG ─────────────────────────────────────────────────────────────

@test "body-size: >400 words triggers BODY_TOO_LONG" {
    write_good_body "$TMP/b.md"
    {
        echo ""
        echo "## Notes"
        echo ""
        # 450 words of filler.
        for i in $(seq 1 450); do printf 'word '; done
        echo ""
    } >> "$TMP/b.md"
    run bash -c "bash '$LINT' '$TMP/b.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "BODY_TOO_LONG"
}

# ── OUTLINE_TOO_LONG ──────────────────────────────────────────────────────────

@test "outline-size: 31 outline lines triggers OUTLINE_TOO_LONG" {
    {
        echo "## Goal"
        echo ""
        echo "Edit \`scripts/x.sh\`."
        echo ""
        echo "## Files to read first"
        echo ""
        echo "- scripts/x.sh"
        echo ""
        echo "## Implementation outline"
        echo ""
        for i in $(seq 1 31); do echo "$i. step number $i"; done
        echo ""
        echo "## Tests required"
        echo ""
        echo "- bats tests/x.bats"
        echo ""
        echo "## Acceptance criteria"
        echo ""
        echo "- [ ] \`bash scripts/x.sh\` exits 0."
        echo ""
        echo "## Verification"
        echo ""
        echo "### Primary smoke test (inner loop)"
        echo ""
        echo '```bash'
        echo "bash scripts/x.sh && echo OK"
        echo '```'
    } > "$TMP/b.md"
    run bash -c "bash '$LINT' '$TMP/b.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "OUTLINE_TOO_LONG"
}

@test "outline-size: 30 outline lines is accepted (no OUTLINE_TOO_LONG)" {
    {
        echo "## Goal"
        echo ""
        echo "Edit \`scripts/x.sh\`."
        echo ""
        echo "## Files to read first"
        echo ""
        echo "- scripts/x.sh"
        echo ""
        echo "## Implementation outline"
        echo ""
        for i in $(seq 1 30); do echo "$i. step number $i"; done
        echo ""
        echo "## Tests required"
        echo ""
        echo "- bats tests/x.bats"
        echo ""
        echo "## Acceptance criteria"
        echo ""
        echo "- [ ] \`bash scripts/x.sh\` exits 0."
        echo ""
        echo "## Verification"
        echo ""
        echo "### Primary smoke test (inner loop)"
        echo ""
        echo '```bash'
        echo "bash scripts/x.sh && echo OK"
        echo '```'
    } > "$TMP/b.md"
    run bash -c "bash '$LINT' '$TMP/b.md' 2>&1"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "OUTLINE_TOO_LONG"
}

# ── relaxed GOAL_NOT_ONE_SENTENCE (review W4) ─────────────────────────────────

@test "goal: 2 sentences and <=30 words passes (no GOAL_NOT_ONE_SENTENCE)" {
    write_good_body "$TMP/b.md"
    sed 's|^Add a deterministic gate.*$|Add a gate to `scripts/lint-issue.sh`. It rejects bodies missing required sections.|' \
        "$TMP/b.md" > "$TMP/b2.md"
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -eq 0 ]
    ! echo "$output" | grep -q "GOAL_NOT_ONE_SENTENCE"
}

@test "goal: 3 sentences triggers GOAL_NOT_ONE_SENTENCE" {
    write_good_body "$TMP/b.md"
    sed 's|^Add a deterministic gate.*$|Add a gate to `scripts/lint-issue.sh`. It rejects bad bodies. It also caps the size.|' \
        "$TMP/b.md" > "$TMP/b2.md"
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "GOAL_NOT_ONE_SENTENCE"
}

@test "goal: >30 words triggers GOAL_NOT_ONE_SENTENCE" {
    write_good_body "$TMP/b.md"
    long_goal="Add a gate to \`scripts/lint-issue.sh\` that one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty twentyone twentytwo twentythree twentyfour twentyfive twentysix twentyseven twentyeight twentynine thirty thirtyone thirtytwo."
    sed "s|^Add a deterministic gate.*\$|$long_goal|" "$TMP/b.md" > "$TMP/b2.md"
    run bash -c "bash '$LINT' '$TMP/b2.md' 2>&1"
    [ "$status" -ge 1 ]
    echo "$output" | grep -q "GOAL_NOT_ONE_SENTENCE"
}

@test "UI_SECTIONS_INCOMPLETE: ui-feature marker without the UI sections is flagged" {
    write_good_body "$TMP/b.md"
    printf '\n<!-- ui-feature -->\n' >> "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'UI_SECTIONS_INCOMPLETE'
    printf '%s\n' "$output" | grep -E 'UI_SECTIONS_INCOMPLETE' | grep -q 'Design reference'
}

@test "UI_SECTIONS_INCOMPLETE: one UI section present requires the other two" {
    write_good_body "$TMP/b.md"
    printf '\n## Interaction states\n\ndefault/hover/focus/loading/empty/error\n' >> "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -ne 0 ]
    out="$(printf '%s\n' "$output" | grep 'UI_SECTIONS_INCOMPLETE')"
    # Inspect only the missing-list portion (before the explanatory parenthetical,
    # which always names all three sections).
    missing="$(printf '%s' "$out" | sed 's/ (UI issues.*//')"
    printf '%s' "$missing" | grep -q 'Design reference'
    printf '%s' "$missing" | grep -q 'UX flows'
    ! printf '%s' "$missing" | grep -q 'Interaction states'
}

@test "UI_SECTIONS_INCOMPLETE: complete UI section set passes" {
    write_good_body "$TMP/b.md"
    printf '\n<!-- ui-feature -->\n\n## Design reference\n\nDESIGN.md#buttons\n\n## Interaction states\n\ndefault/hover/focus/loading/empty/error/disabled\n\n## UX flows\n\nhappy: click -> submit; failure: 500 -> toast; edge: empty list\n' >> "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'UI_SECTIONS_INCOMPLETE'
}

@test "UI_SECTIONS_INCOMPLETE: non-UI issue (no marker, no UI sections) is not flagged" {
    write_good_body "$TMP/b.md"
    run bash "$LINT" "$TMP/b.md"
    [ "$status" -eq 0 ]
    ! printf '%s\n' "$output" | grep -q 'UI_SECTIONS_INCOMPLETE'
}
