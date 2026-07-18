#!/usr/bin/env bats
# tests/unit/persona-overlay-select.bats — per-issue persona archetype overlays
#
# Coverage:
#   - issue labels select bundled security/performance archetypes
#   - autonomous-persona-synth composes an issue overlay into the effective persona only
#   - match seam errors fail open to the base persona
#   - self-grow writes one user-state archetype and reuses it for a near duplicate

if [ -z "${BATS_TEST_DIRNAME:-}" ]; then
    exec bats "$0" "$@"
fi

bats_require_minimum_version 1.5.0

SCRIPT_DIR="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
PERSONA_CATALOG="$SCRIPT_DIR/scripts/persona-catalog.sh"
PERSONA_SYNTH="$SCRIPT_DIR/scripts/autonomous-persona-synth.sh"

setup() {
    TMP="$(mktemp -d -t test-persona-overlay.XXXXXX)"
    export HOME="$TMP/home"
    AUTOSPEC_HOME="$HOME/.autospec"
    REPO_UNDER_TEST="$TMP/repo"
    mkdir -p "$AUTOSPEC_HOME/personas" "$REPO_UNDER_TEST/.autospec"

    BASE_PERSONA="$TMP/base-persona.md"
    cat > "$BASE_PERSONA" <<'BASE'
# Operator persona

## Decision style

Prefer small, verified changes.
BASE
}

teardown() {
    rm -rf "$TMP"
}

select_overlay() {
    run env HOME="$HOME" bash "$PERSONA_CATALOG" select-overlay "$@"
}

run_synth() {
    run env \
        HOME="$HOME" \
        AUTOSPEC_PERSONA_SYNTH_CMD="cat '$BASE_PERSONA'" \
        "$@" \
        bash "$PERSONA_SYNTH" --repo-root "$REPO_UNDER_TEST" --autospec-home "$AUTOSPEC_HOME" --force
}

run_synth_no_force() {
    run env \
        HOME="$HOME" \
        AUTOSPEC_PERSONA_SYNTH_CMD="exit 99" \
        "$@" \
        bash "$PERSONA_SYNTH" --repo-root "$REPO_UNDER_TEST" --autospec-home "$AUTOSPEC_HOME"
}

@test "security-labeled issue selects the security hardener overlay" {
    select_overlay --title "Rotate API credentials safely" --body "Tighten token validation." --labels "security,bug"

    [ "$status" -eq 0 ]
    [ "$output" = "security-hardener" ]
}

@test "perf-labeled issue selects the performance engineer overlay" {
    select_overlay --title "Reduce queue startup cost" --body "Bound repeated work." --labels "perf,optimization"

    [ "$status" -eq 0 ]
    [ "$output" = "performance-engineer" ]
}

@test "synth composes an issue overlay without changing the base persona doctrine" {
    run_synth \
        AUTOSPEC_PERSONA_ISSUE_TITLE="Validate secret handling" \
        AUTOSPEC_PERSONA_ISSUE_BODY="Credential parsing crosses a trust boundary." \
        AUTOSPEC_PERSONA_ISSUE_LABELS="security"

    [ "$status" -eq 0 ]
    grep -q "Security Hardener" "$REPO_UNDER_TEST/.autospec/operator-persona.effective.md"
    grep -q "Issue archetype overlay" "$REPO_UNDER_TEST/.autospec/operator-persona.effective.md"
    if grep -q "Security Hardener" "$AUTOSPEC_HOME/operator-persona.md"; then
        echo "global base persona should not be mutated by issue overlay" >&2
        return 1
    fi
}

@test "fresh global persona still refreshes the current issue overlay" {
    cp "$BASE_PERSONA" "$AUTOSPEC_HOME/operator-persona.md"

    run_synth_no_force \
        AUTOSPEC_PERSONA_ISSUE_TITLE="Validate secret handling" \
        AUTOSPEC_PERSONA_ISSUE_BODY="Credential parsing crosses a trust boundary." \
        AUTOSPEC_PERSONA_ISSUE_LABELS="security"

    [ "$status" -eq 0 ]
    grep -q "global persona fresh" <<<"$output"
    grep -q "Security Hardener" "$REPO_UNDER_TEST/.autospec/operator-persona.effective.md"
    grep -q "Issue archetype overlay" "$REPO_UNDER_TEST/.autospec/operator-persona.effective.md"
}

@test "fresh global persona discards stale issue overlays when no issue is active" {
    cp "$BASE_PERSONA" "$AUTOSPEC_HOME/operator-persona.md"
    mkdir -p "$REPO_UNDER_TEST/.autospec"
    cat > "$REPO_UNDER_TEST/.autospec/operator-persona.effective.md" <<'STALE'
# Operator persona

## Issue archetype overlay

# Security Hardener
STALE

    run_synth_no_force

    [ "$status" -eq 0 ]
    grep -q "global persona fresh" <<<"$output"
    if grep -q "Issue archetype overlay" "$REPO_UNDER_TEST/.autospec/operator-persona.effective.md"; then
        echo "stale issue overlay should be removed when no issue is active" >&2
        return 1
    fi
}

@test "match seam errors fail open to the base persona" {
    run_synth \
        AUTOSPEC_PERSONA_MATCH_CMD="exit 42" \
        AUTOSPEC_PERSONA_ISSUE_TITLE="Validate secret handling" \
        AUTOSPEC_PERSONA_ISSUE_BODY="Credential parsing crosses a trust boundary." \
        AUTOSPEC_PERSONA_ISSUE_LABELS="security"

    [ "$status" -eq 0 ]
    grep -q "# Operator persona" "$REPO_UNDER_TEST/.autospec/operator-persona.effective.md"
    if grep -q "Security Hardener" "$REPO_UNDER_TEST/.autospec/operator-persona.effective.md"; then
        echo "failed match seam should not fall through to default matching" >&2
        return 1
    fi
}

@test "near-duplicate follow-up issue reuses a generated user archetype" {
    select_overlay --title "Add xylophone quasar zebra planner" --body "Niche conductors need a specific overlay." --labels ""
    [ "$status" -eq 0 ]
    first_id="$output"
    [ -n "$first_id" ]
    [ -f "$AUTOSPEC_HOME/personas/${first_id}.md" ]
    [ ! -f "$SCRIPT_DIR/personas/catalog/${first_id}.md" ]

    select_overlay --title "Xylophone quasar zebra planner follow-up" --body "Reuse the same niche overlay." --labels ""
    [ "$status" -eq 0 ]
    [ "$output" = "$first_id" ]

    generated_count="$(find "$AUTOSPEC_HOME/personas" -maxdepth 1 -type f -name '*.md' | wc -l | tr -d ' ')"
    [ "$generated_count" = "1" ]
}
