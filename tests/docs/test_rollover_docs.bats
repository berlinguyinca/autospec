#!/usr/bin/env bats
# tests/docs/test_rollover_docs.bats
#
# Validates README.md and AGENTS.md documentation for auto context rollover
# feature as required by issue #754.
#
# Run: bats tests/docs/test_rollover_docs.bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
README="$REPO_ROOT/README.md"
AGENTS="$REPO_ROOT/AGENTS.md"

@test "test_readme_has_auto_context_rollover_heading" {
  grep -q '^## Auto context rollover (opt-in)' "$README"
}

@test "test_readme_lists_at_least_3_escape_hatches" {
  count=$(grep -c -- '--disable-auto-rollover\|AUTOSPEC_AUTO_ROLLOVER=0\|command claude\|no-auto-rollover\.flag\|--no-monitor' "$README")
  [ "$count" -ge 3 ]
}

@test "test_agents_md_mentions_autospec_rollover_status" {
  grep -q '/autospec-rollover-status' "$AGENTS"
}

@test "test_readme_links_to_spec_path" {
  grep -q 'docs/specs/2026-05-31-auto-context-rollover-design\.md' "$README"
}
