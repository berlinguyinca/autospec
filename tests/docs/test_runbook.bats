#!/usr/bin/env bats
# tests/docs/test_runbook.bats
#
# Validates docs/runbooks/auto-context-rollover-e2e.md for structure and
# content required by issue #753.
#
# Run: bats tests/docs/test_runbook.bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
RUNBOOK="$REPO_ROOT/docs/runbooks/auto-context-rollover-e2e.md"

@test "test_runbook_file_exists" {
  [ -f "$RUNBOOK" ]
}

@test "test_runbook_has_three_scenario_headings" {
  count=$(grep -c '^## Scenario [123]' "$RUNBOOK")
  [ "$count" -eq 3 ]
}

@test "test_runbook_mentions_handoff_file_path" {
  grep -q '\.turbo/handoff' "$RUNBOOK"
}

@test "test_runbook_mentions_pct_thresholds_50_and_80" {
  grep -q '50%' "$RUNBOOK"
  grep -q '80%' "$RUNBOOK"
}
