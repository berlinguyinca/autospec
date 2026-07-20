#!/usr/bin/env bats
# tests/autonomous/test_persona_sources.bats
# TDD contract for scripts/autonomous-persona-sources.sh
#
# Coverage:
#   - Output structure (top-level keys, per-entry fields)
#   - Class routing: repo-local sources → overlay; operator-level sources → global
#   - Precedence ordering within each sub-bundle
#   - Per-repo .autospec/persona-overlay.md lands in overlay, not global
#   - Missing sources are silently skipped (not fatal); empty run exits 0
#
# Engineering notes:
#   - All fixtures written to real temp files; no process substitutions with [ -f ]
#     (macOS bash 3.2: [ -f <(...) ] is always false)
#   - .autospec/persona-overlay.md is gitignored; materialized at runtime
#   - Negation via jq count == 0, never via ! piped grep
#   - jq capture()/== patterns used; no test() with interpolated values

SCRIPT_DIR="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
PERSONA_SOURCES="$SCRIPT_DIR/scripts/autonomous-persona-sources.sh"

setup() {
    TMP="$(mktemp -d -t test-persona-sources.XXXXXX)"

    FAKE_REPO="$TMP/repo"
    mkdir -p "$FAKE_REPO/docs/memory"
    mkdir -p "$FAKE_REPO/docs"
    mkdir -p "$FAKE_REPO/.autospec"

    FAKE_AUTOSPEC="$TMP/autospec-home"
    mkdir -p "$FAKE_AUTOSPEC"

    export REPO_ROOT="$FAKE_REPO"
    export AUTOSPEC_HOME="$FAKE_AUTOSPEC"
}

teardown() {
    rm -rf "$TMP"
}

# ---------------------------------------------------------------------------
# Helper: run_sources uses env vars set in setup / individual tests
# ---------------------------------------------------------------------------
run_sources() {
    run bash "$PERSONA_SOURCES"
}

# Helper: count occurrences of source value in bundle key
count_source_in() {
    local bundle="$1"   # "global" or "overlay"
    local src="$2"
    echo "$output" | jq --arg bundle "$bundle" --arg src "$src" \
      '.[$bundle] | map(select(.source == $src)) | length'
}

# ---------------------------------------------------------------------------
# Output structure
# ---------------------------------------------------------------------------

@test "script is executable and exists" {
    [ -f "$PERSONA_SOURCES" ]
    [ -x "$PERSONA_SOURCES" ]
}

@test "empty run: exits 0 with global and overlay arrays" {
    run_sources
    [ "$status" -eq 0 ]
    echo "$output" | jq -e 'has("global") and has("overlay")' > /dev/null
}

@test "empty run: both arrays are empty when no sources present" {
    run_sources
    [ "$status" -eq 0 ]
    local _gc _oc
    _gc="$(echo "$output" | jq '.global | length')"
    _oc="$(echo "$output" | jq '.overlay | length')"
    [ "$_gc" -eq 0 ]
    [ "$_oc" -eq 0 ]
}

@test "each present entry has source, precedence, path, present fields" {
    # Materialize a real file so at least one entry is present
    echo '{"version":1,"batches":[]}' > "$FAKE_AUTOSPEC/operator-persona.answers.json"
    run_sources
    [ "$status" -eq 0 ]
    echo "$output" | jq -e \
      '.global[0] | has("source") and has("precedence") and has("path") and has("present")' \
      > /dev/null
}

@test "present field is boolean true for found files" {
    echo '{"version":1}' > "$FAKE_AUTOSPEC/operator-persona.answers.json"
    run_sources
    [ "$status" -eq 0 ]
    local _present
    _present="$(echo "$output" | jq '.global[0].present')"
    [ "$_present" = "true" ]
}

# ---------------------------------------------------------------------------
# Class routing: operator-level sources → global
# ---------------------------------------------------------------------------

@test "interview answers land in global, not overlay" {
    echo '{"version":1,"batches":[]}' > "$FAKE_AUTOSPEC/operator-persona.answers.json"
    run_sources
    [ "$status" -eq 0 ]
    local _in_global _in_overlay
    _in_global="$(count_source_in global interview-answers)"
    _in_overlay="$(count_source_in overlay interview-answers)"
    [ "$_in_global" -ge 1 ]
    [ "$_in_overlay" -eq 0 ]
}

@test "mined digest lands in global, not overlay" {
    echo '{}' > "$FAKE_AUTOSPEC/persona-mined-digest.json"
    run_sources
    [ "$status" -eq 0 ]
    local _in_global _in_overlay
    _in_global="$(count_source_in global mined-digest)"
    _in_overlay="$(count_source_in overlay mined-digest)"
    [ "$_in_global" -ge 1 ]
    [ "$_in_overlay" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Class routing: repo-local sources → overlay
# ---------------------------------------------------------------------------

@test "docs/memory feedback_* files land in overlay, not global" {
    # Write a real temp file (not process sub)
    echo "# feedback note" > "$FAKE_REPO/docs/memory/feedback_test.md"
    run_sources
    [ "$status" -eq 0 ]
    local _in_overlay _in_global
    _in_overlay="$(count_source_in overlay repo-memory)"
    _in_global="$(count_source_in global repo-memory)"
    [ "$_in_overlay" -ge 1 ]
    [ "$_in_global" -eq 0 ]
}

@test "docs/memory project_* files land in overlay, not global" {
    echo "# project note" > "$FAKE_REPO/docs/memory/project_foo.md"
    run_sources
    [ "$status" -eq 0 ]
    local _in_overlay _in_global
    _in_overlay="$(count_source_in overlay repo-memory)"
    _in_global="$(count_source_in global repo-memory)"
    [ "$_in_overlay" -ge 1 ]
    [ "$_in_global" -eq 0 ]
}

@test "AGENTS.md lands in overlay, not global" {
    echo "# agents" > "$FAKE_REPO/AGENTS.md"
    run_sources
    [ "$status" -eq 0 ]
    local _in_overlay _in_global
    _in_overlay="$(count_source_in overlay agents-md)"
    _in_global="$(count_source_in global agents-md)"
    [ "$_in_overlay" -ge 1 ]
    [ "$_in_global" -eq 0 ]
}

@test "docs/AUTONOMY-CHARTER.md lands in overlay, not global" {
    echo "# charter" > "$FAKE_REPO/docs/AUTONOMY-CHARTER.md"
    run_sources
    [ "$status" -eq 0 ]
    local _in_overlay _in_global
    _in_overlay="$(count_source_in overlay autonomy-charter)"
    _in_global="$(count_source_in global autonomy-charter)"
    [ "$_in_overlay" -ge 1 ]
    [ "$_in_global" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Per-repo overlay (.autospec/persona-overlay.md — gitignored, runtime-only)
# ---------------------------------------------------------------------------

@test "per-repo overlay file lands in overlay bundle, not global" {
    # Materialize at runtime (gitignored; never committed)
    echo "# per-repo persona override" > "$FAKE_REPO/.autospec/persona-overlay.md"
    run_sources
    [ "$status" -eq 0 ]
    local _in_overlay _in_global
    _in_overlay="$(count_source_in overlay per-repo-overlay)"
    _in_global="$(count_source_in global per-repo-overlay)"
    [ "$_in_overlay" -ge 1 ]
    [ "$_in_global" -eq 0 ]
}

@test "absent per-repo overlay is silently skipped, not fatal" {
    # No .autospec/persona-overlay.md in FAKE_REPO
    run_sources
    [ "$status" -eq 0 ]
    local _cnt
    _cnt="$(count_source_in overlay per-repo-overlay)"
    [ "$_cnt" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Missing sources — skipped not fatal
# ---------------------------------------------------------------------------

@test "all optional sources absent: exits 0" {
    run_sources
    [ "$status" -eq 0 ]
}

@test "only AGENTS.md present: overlay has one entry, global is empty" {
    echo "# agents" > "$FAKE_REPO/AGENTS.md"
    run_sources
    [ "$status" -eq 0 ]
    local _oc _gc
    _oc="$(echo "$output" | jq '.overlay | length')"
    _gc="$(echo "$output" | jq '.global | length')"
    [ "$_oc" -eq 1 ]
    [ "$_gc" -eq 0 ]
}

@test "only interview answers present: global has one entry, overlay is empty" {
    echo '{"version":1}' > "$FAKE_AUTOSPEC/operator-persona.answers.json"
    run_sources
    [ "$status" -eq 0 ]
    local _gc _oc
    _gc="$(echo "$output" | jq '.global | length')"
    _oc="$(echo "$output" | jq '.overlay | length')"
    [ "$_gc" -eq 1 ]
    [ "$_oc" -eq 0 ]
}

# ---------------------------------------------------------------------------
# Precedence ordering
# ---------------------------------------------------------------------------

@test "global entries sorted ascending by precedence: interview(0) before mined(3)" {
    echo '{"version":1}' > "$FAKE_AUTOSPEC/operator-persona.answers.json"
    echo '{}'            > "$FAKE_AUTOSPEC/persona-mined-digest.json"
    run_sources
    [ "$status" -eq 0 ]
    local _first _last
    _first="$(echo "$output" | jq '.global[0].precedence')"
    _last="$(echo  "$output" | jq '.global[-1].precedence')"
    [ "$_first" -eq 0 ]
    [ "$_last"  -eq 3 ]
}

@test "overlay entries: repo-memory(1) entries all precede autonomy-charter(2)" {
    echo "# feedback" > "$FAKE_REPO/docs/memory/feedback_a.md"
    echo "# charter"  > "$FAKE_REPO/docs/AUTONOMY-CHARTER.md"
    run_sources
    [ "$status" -eq 0 ]
    # All prec-1 entries must appear before prec-2 entries in the sorted array
    local _max1 _min2
    _max1="$(echo "$output" | jq \
      '[.overlay[] | select(.precedence == 1)] | map(.precedence) | max // -1')"
    _min2="$(echo "$output" | jq \
      '[.overlay[] | select(.precedence == 2)] | map(.precedence) | min // 999')"
    [ "$_max1" -le "$_min2" ]
}

@test "overlay sorted: multiple feedback files all at precedence 1" {
    echo "# fb1" > "$FAKE_REPO/docs/memory/feedback_one.md"
    echo "# fb2" > "$FAKE_REPO/docs/memory/feedback_two.md"
    echo "# pr1" > "$FAKE_REPO/docs/memory/project_one.md"
    run_sources
    [ "$status" -eq 0 ]
    local _cnt
    _cnt="$(echo "$output" | jq \
      '[.overlay[] | select(.source == "repo-memory" and .precedence == 1)] | length')"
    [ "$_cnt" -eq 3 ]
}

# ---------------------------------------------------------------------------
# Path values in output
# ---------------------------------------------------------------------------

@test "interview answers entry records absolute path" {
    local _fixture="$FAKE_AUTOSPEC/operator-persona.answers.json"
    echo '{"version":1}' > "$_fixture"
    run_sources
    [ "$status" -eq 0 ]
    local _path
    _path="$(echo "$output" | jq -r \
      '.global[] | select(.source == "interview-answers") | .path')"
    [ "$_path" = "$_fixture" ]
}

@test "repo-memory entry path points to the actual file" {
    local _fixture="$FAKE_REPO/docs/memory/feedback_check.md"
    echo "# check" > "$_fixture"
    run_sources
    [ "$status" -eq 0 ]
    local _found
    _found="$(echo "$output" | jq --arg p "$_fixture" \
      '[.overlay[] | select(.path == $p)] | length')"
    [ "$_found" -eq 1 ]
}

# ---------------------------------------------------------------------------
# Code-aware gather (issue #1727)
# ---------------------------------------------------------------------------

@test "code-aware: CLAUDE.md alone yields a non-empty overlay bundle" {
    printf '# Project rules\nUse Decimal for money.\n' > "$FAKE_REPO/CLAUDE.md"
    run_sources
    [ "$status" -eq 0 ]
    [ "$(count_source_in overlay agent-instructions)" -eq 1 ]
    [ "$(echo "$output" | jq '.meta.source_count')" -gt 0 ]
}

@test "code-aware: docs/specs markdown files are gathered as design-spec" {
    mkdir -p "$FAKE_REPO/docs/specs"
    printf '# Spec A\nintent\n' > "$FAKE_REPO/docs/specs/a.md"
    printf '# Spec B\nintent\n' > "$FAKE_REPO/docs/specs/b.md"
    run_sources
    [ "$status" -eq 0 ]
    [ "$(count_source_in overlay design-spec)" -eq 2 ]
}

@test "code-aware: binary files are excluded from the gather" {
    mkdir -p "$FAKE_REPO/docs/specs"
    printf '\x00\x01\x02binary\x00' > "$FAKE_REPO/docs/specs/blob.md"
    printf '# real spec\n' > "$FAKE_REPO/docs/specs/real.md"
    run_sources
    [ "$status" -eq 0 ]
    [ "$(count_source_in overlay design-spec)" -eq 1 ]
}

@test "code-aware: per-source byte cap skips oversized files" {
    export AUTOSPEC_PERSONA_SOURCE_MAX_BYTES=8
    printf 'this content is definitely longer than eight bytes\n' > "$FAKE_REPO/CLAUDE.md"
    run_sources
    [ "$status" -eq 0 ]
    [ "$(count_source_in overlay agent-instructions)" -eq 0 ]
}

@test "code-aware: root stack manifests are gathered as stack-manifest" {
    printf '[package]\nname = "x"\n' > "$FAKE_REPO/Cargo.toml"
    run_sources
    [ "$status" -eq 0 ]
    [ "$(count_source_in overlay stack-manifest)" -eq 1 ]
}

@test "code-aware: interview answers stay precedence 0 above code sources" {
    printf '{"decision_style":"cautious"}\n' > "$FAKE_AUTOSPEC/operator-persona.answers.json"
    printf '# rules\n' > "$FAKE_REPO/CLAUDE.md"
    run_sources
    [ "$status" -eq 0 ]
    [ "$(echo "$output" | jq '.global[] | select(.source=="interview-answers") | .precedence')" -eq 0 ]
    [ "$(echo "$output" | jq '.overlay[] | select(.source=="agent-instructions") | .precedence')" -ge 1 ]
}
