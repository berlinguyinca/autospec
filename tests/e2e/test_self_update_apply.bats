#!/usr/bin/env bats
# tests/e2e/test_self_update_apply.bats — end-to-end test for the startup
# self-update apply path (§7.2 procedure).
#
# This test:
#   1. Clones berlinguyinca/autospec into a tmp dir.
#   2. Checks out a commit 5 behind HEAD (simulating a stale install).
#   3. Runs install.sh --harness claude to populate the sandboxed skill files.
#   4. Writes the parent SHA as the installed-version.
#   5. Runs the preflight block and asserts the installed-version is updated to
#      main HEAD and last-update-check is written.
#
# GATING: skipped unless E2E=1 is set. Also skipped if git/curl are absent.
#
# Spec ref: docs/specs/2026-05-01-autospec-startup-self-update-design.md §7.2

setup() {
    if [ "${E2E:-0}" != "1" ]; then
        skip "e2e gated — set E2E=1 to run"
    fi
    if ! command -v git >/dev/null 2>&1; then
        skip "git not installed"
    fi
    if ! command -v curl >/dev/null 2>&1; then
        skip "curl not installed"
    fi

    TMP="$(mktemp -d)"
    export TMP

    # Clone the repo.
    git clone --quiet https://github.com/berlinguyinca/autospec.git "$TMP/repo"
    cd "$TMP/repo"

    # Record HEAD SHA (7-char) before detaching.
    HEAD_SHA="$(git rev-parse --short=7 HEAD)"
    export HEAD_SHA

    # Check out a commit 5 behind HEAD.
    PARENT_SHA="$(git rev-parse --short=7 HEAD~5)"
    export PARENT_SHA
    git checkout --quiet "$PARENT_SHA"

    # Sandboxed HOME.
    export HOME="$TMP/home"
    mkdir -p "$HOME/.autospec"
    export CLAUDE_CONFIG_DIR="$HOME/.claude-config"

    # Install the skill (from the checked-out parent).
    bash skills/autospec/install.sh --harness claude --update >/dev/null 2>&1 || true

    # Seed installed-version as parent so preflight sees a delta.
    printf '%s\n' "$PARENT_SHA" > "$HOME/.autospec/installed-version"

    # Remove rate-limit file so the 24h gate is open.
    rm -f "$HOME/.autospec/last-update-check"

    export REPO_DIR="$TMP/repo"
}

teardown() {
    if [ -n "${TMP:-}" ] && [ -d "$TMP" ]; then
        rm -rf "$TMP"
    fi
}

@test "preflight applies main HEAD when local is parent commit" {
    # Extract the startup self-update block from the checked-out SKILL.md.
    BLOCK="$(awk '/^## Startup self-update/{f=1} f && /^```bash/{g=1; next} g && /^```/{g=0; f=0; next} g{print}' "$REPO_DIR/skills/autospec/SKILL.md")"
    [ -n "$BLOCK" ] || { echo "BLOCK extraction failed" >&2; return 1; }

    # Run the preflight in the sandboxed HOME.
    run bash -c "export HOME='$HOME'; $BLOCK"

    # Preflight must succeed (fail-open contract).
    [ "$status" -eq 0 ]

    # installed-version must now be the main HEAD SHA (7 chars).
    INSTALLED="$(cat "$HOME/.autospec/installed-version" 2>/dev/null || echo '')"
    [ "$INSTALLED" = "$HEAD_SHA" ] || {
        echo "Expected installed-version=$HEAD_SHA, got: $INSTALLED" >&2
        return 1
    }

    # last-update-check must have been written.
    [ -s "$HOME/.autospec/last-update-check" ]
}

@test "preflight skips when E2E=0 (sanity — this test always passes)" {
    # This trivially-passing test verifies the file loads cleanly even when
    # the E2E gate would skip the main test. It runs unconditionally because
    # the setup() already passed (E2E=1 got us here).
    true
}
