#!/usr/bin/env bash
# scripts/gen-issue-skeleton.sh — render a structured YAML input into a team-lensed issue body.
#
# Thin wrapper over the Rust implementation (`autospec issue-skeleton`,
# autospec_core::issue_skeleton). The parsing, rendering, and linting now live
# in Rust (issue #4440); this script is kept for one release so existing
# callers (the /autospec-define and /autospec-split skills) keep working.
#
# Usage:
#   scripts/gen-issue-skeleton.sh --input <file>   # read YAML from file
#   scripts/gen-issue-skeleton.sh                  # read YAML from stdin
#   scripts/gen-issue-skeleton.sh --help           # show this help
#
# Required YAML keys:
#   issue_id, spec_path, spec_url, goal_sentence,
#   team_personality (list), review_counter_team (list), files_to_read (list),
#   files_touched (list), local_llm_notes (list), dependencies (list),
#   implementation_scope (list), out_of_scope (list),
#   implementation_outline_lines (list), tests_required (list),
#   acceptance_criteria (list), verification.primary_smoke,
#   branch_name
# Optional keys: verification.operator_full (falls back to primary_smoke),
#   implementation_surface (the crate and module the work belongs in, #4439).
# Optional profile: feature_profile: security_database additionally requires
#   evidence_consumed, controls_covered, prerequisites (lists).
#
# Output: structured markdown issue body on stdout (only when lint passes).
# The body is linted in-process by the Rust core (autospec_core::lint); the
# shell lint-issue.sh is no longer invoked.
#
# Exit codes:
#   0   — success (no blocking lint findings; body on stdout)
#   1   — MISSING_FIELD:<key> or YAML parse error (on stderr)
#   N   — N blocking lint findings (on stderr)

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Resolve the autospec binary the same way the other drain scripts do: prefer
# a locally-built debug binary, then the one on PATH.
if [ -x "$REPO_ROOT/target/debug/autospec" ]; then
  AUTOSPEC_BIN="$REPO_ROOT/target/debug/autospec"
elif command -v autospec >/dev/null 2>&1; then
  AUTOSPEC_BIN="$(command -v autospec)"
else
  echo "gen-issue-skeleton.sh: autospec binary not found (build it with 'cargo build -p autospec-cli' or put 'autospec' on PATH)" >&2
  exit 1
fi

exec "$AUTOSPEC_BIN" issue-skeleton "$@"
