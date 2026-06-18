#!/usr/bin/env bats
# tests/harmonize/test_harmonize_e2e.bats
#
# End-to-end integration test for harmonize.sh orchestrator.
# Runs the full 5-stage pipeline against the messy-app fixture with
# AUTOSPEC_HARMONIZE_LLM_STUB=1 (deterministic, no LLM calls) and
# AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1 (swatch fallback for preview).
#
# All node/jq dependencies are guarded with "command -v || skip".

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  ORCHESTRATOR="$REPO_ROOT/skills/autospec-harmonize/scripts/harmonize.sh"
  FIXTURE_DIR="$REPO_ROOT/skills/autospec-harmonize/test-targets/messy-app"
  TEST_TMPDIR="$(mktemp -d /tmp/autospec-harmonize-e2e-XXXXXX)"
  export AUTOSPEC_HARMONIZE_LLM_STUB=1
  export AUTOSPEC_HARMONIZE_NO_PLAYWRIGHT=1
  # Point stage scripts at sibling dir (not ~/.autospec/scripts)
  export AUTOSPEC_SCRIPTS_DIR="$REPO_ROOT/skills/autospec-harmonize/scripts"
}

teardown() {
  rm -rf "$TEST_TMPDIR"
}

# ---------------------------------------------------------------------------
# Guard helpers
# ---------------------------------------------------------------------------

require_node() {
  command -v node >/dev/null 2>&1 || skip "node not available"
}

require_jq() {
  command -v jq >/dev/null 2>&1 || skip "jq not available"
}

# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

@test "harmonize.sh exists and is executable" {
  [ -f "$ORCHESTRATOR" ]
  [ -x "$ORCHESTRATOR" ]
}

@test "harmonize.sh --source-only --no-live-preview exits 0" {
  require_node
  require_jq
  run bash "$ORCHESTRATOR" \
    --root "$FIXTURE_DIR" \
    --source-only \
    --no-live-preview \
    --variants "minimal,high-contrast" \
    --out "$TEST_TMPDIR"
  echo "STATUS: $status"
  echo "OUTPUT: $output"
  [ "$status" -eq 0 ]
}

@test "harmonize.sh writes discovered-tokens.json with source==source" {
  require_node
  require_jq
  bash "$ORCHESTRATOR" \
    --root "$FIXTURE_DIR" \
    --source-only \
    --no-live-preview \
    --variants "minimal,high-contrast" \
    --out "$TEST_TMPDIR"
  [ -f "$TEST_TMPDIR/discovered-tokens.json" ]
  val="$(jq -r '.source' "$TEST_TMPDIR/discovered-tokens.json")"
  [ "$val" = "source" ]
}

@test "harmonize.sh writes variants array with >=3 elements" {
  require_node
  require_jq
  bash "$ORCHESTRATOR" \
    --root "$FIXTURE_DIR" \
    --source-only \
    --no-live-preview \
    --variants "minimal,high-contrast" \
    --out "$TEST_TMPDIR"
  [ -f "$TEST_TMPDIR/variants.json" ]
  count="$(jq 'length' "$TEST_TMPDIR/variants.json")"
  [ "$count" -ge 3 ]
}

@test "harmonize.sh writes preview/index.html" {
  require_node
  require_jq
  bash "$ORCHESTRATOR" \
    --root "$FIXTURE_DIR" \
    --source-only \
    --no-live-preview \
    --variants "minimal,high-contrast" \
    --out "$TEST_TMPDIR"
  [ -f "$TEST_TMPDIR/preview/index.html" ]
}

@test "harmonize.sh writes a spec file under docs/specs/" {
  require_node
  require_jq
  bash "$ORCHESTRATOR" \
    --root "$FIXTURE_DIR" \
    --source-only \
    --no-live-preview \
    --variants "minimal,high-contrast" \
    --out "$TEST_TMPDIR"
  # Find any file matching the expected pattern
  spec_count="$(find "$TEST_TMPDIR/docs/specs" -name "*.md" 2>/dev/null | wc -l | tr -d ' ')"
  [ "$spec_count" -ge 1 ]
}
