#!/usr/bin/env bash
# autospec-initiative-contract.sh — Contract check for the Initiative layer.
#
# The Initiative model spans a Rust core module, a CLI surface, a JSON schema,
# and a design document. Each of those repeats a small vocabulary — role names,
# task states, coverage states, capability names, subcommands — and the four
# drift apart silently. This check fails when they do.
#
# Usage:
#   bash scripts/autospec-initiative-contract.sh [--repo-root DIR]
#
# Exit codes:
#   0  Every contract holds
#   1  At least one contract is broken

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo-root)
      REPO_ROOT="$2"
      shift 2
      ;;
    -h|--help)
      sed -n '2,15p' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      printf 'unknown option: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

CORE_DIR="$REPO_ROOT/crates/autospec-core/src/initiative"
CLI_FILE="$REPO_ROOT/crates/autospec-cli/src/commands/initiative.rs"
COMMANDS_FILE="$REPO_ROOT/crates/autospec-cli/src/commands/mod.rs"
SCHEMA_FILE="$REPO_ROOT/schemas/autospec-initiative.schema.json"
DESIGN_FILE="$REPO_ROOT/docs/specs/2026-09-03-initiative-planning-multi-repo-orchestration-v2-design.md"
CLI_TEST_FILE="$REPO_ROOT/crates/autospec-cli/tests/initiative_commands.rs"

FAILURES=0

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  FAILURES=$((FAILURES + 1))
}

require_file() {
  if [[ ! -f "$1" ]]; then
    fail "missing required file: ${1#"$REPO_ROOT"/}"
    return 1
  fi
  return 0
}

require_in_file() {
  local file="$1" needle="$2" label="$3"
  if [[ ! -f "$file" ]]; then
    fail "$label: ${file#"$REPO_ROOT"/} does not exist"
    return
  fi
  if ! grep -qF -- "$needle" "$file"; then
    fail "$label: ${file#"$REPO_ROOT"/} does not mention '$needle'"
  fi
}

# 1. Every core module the design document names exists.
for module in ids repository definition plan task dag roles routing dispatch traceability projection store; do
  require_file "$CORE_DIR/$module.rs" || continue
  require_in_file "$CORE_DIR/mod.rs" "pub mod $module;" "core module registration"
  require_in_file "$DESIGN_FILE" "initiative::$module" "design module map"
done

require_file "$CORE_DIR/mod.rs" || true
require_in_file "$REPO_ROOT/crates/autospec-core/src/lib.rs" "pub mod initiative;" "core crate registration"

# 2. The CLI exposes every documented subcommand and routes to it.
require_file "$CLI_FILE" || true
require_in_file "$COMMANDS_FILE" '"initiative" => initiative::run(rest),' "CLI dispatch"
for subcommand in init validate ready coverage verify project status; do
  require_in_file "$CLI_FILE" "\"$subcommand\" =>" "CLI subcommand"
  require_in_file "$DESIGN_FILE" "autospec initiative $subcommand" "documented subcommand"
done

# 3. Task states are identical in the core, the schema, and the design document.
for state in DEFINED BLOCKED READY LEASED RUNNING AWAITING_TEST AWAITING_REVIEW \
  CHANGES_REQUESTED FAILED_RETRYABLE FAILED_TERMINAL SUPERSEDED VERIFIED; do
  require_in_file "$CORE_DIR/task.rs" "\"$state\"" "task state"
  require_in_file "$SCHEMA_FILE" "\"$state\"" "schema task state"
done

# 4. Agent roles are identical in the core and the schema.
for role in spec-author architect task-planner implementer test-engineer \
  ux-reviewer reviewer spec-verifier; do
  require_in_file "$CORE_DIR/roles.rs" "\"$role\"" "agent role"
  require_in_file "$SCHEMA_FILE" "\"$role\"" "schema agent role"
done

# 5. Repository capabilities are identical in the core and the schema.
for capability in read issues branches push pull_requests workflows \
  project_mutation administration; do
  require_in_file "$CORE_DIR/repository.rs" "\"$capability\"" "repository capability"
  require_in_file "$SCHEMA_FILE" "\"$capability\"" "schema repository capability"
done

# 6. Coverage states are identical in the core and the schema.
for state in defined planned in_progress implemented tested reviewed verified \
  failed blocked waived; do
  require_in_file "$CORE_DIR/traceability.rs" "\"$state\"" "coverage state"
  require_in_file "$SCHEMA_FILE" "\"$state\"" "schema coverage state"
done

# 7. The registry layout in the design document matches the store.
for artifact in "initiative.json" "definition/definition-v" "workspace/repositories.json" \
  "plans/architecture-plan-v" "graph/task-graph-v" "verification/requirements-matrix.json" \
  "projections/github.json" "audit/events.jsonl"; do
  require_in_file "$CORE_DIR/store.rs" "$artifact" "store layout"
  require_in_file "$DESIGN_FILE" "$artifact" "documented layout"
done

# 8. Money in this subsystem stays on integers.
if grep -qE '\bf64\b' "$CORE_DIR"/*.rs; then
  fail "initiative modules must not use f64; costs are integer millicents"
fi

# 9. The acceptance criteria table stays complete.
for criterion in $(seq 1 18); do
  if ! grep -qE "^\| $criterion \|" "$DESIGN_FILE"; then
    fail "design document has no row for acceptance criterion $criterion"
  fi
done

# 10. The end-to-end fixture keeps spanning three repositories and two owners.
require_file "$CLI_TEST_FILE" || true
require_in_file "$CLI_TEST_FILE" "github.com/InferWeave/autospec" "multi-repository fixture"
require_in_file "$CLI_TEST_FILE" "github.com/InferWeave/autospec-orchestrator" "multi-repository fixture"
require_in_file "$CLI_TEST_FILE" "github.com/OtherOrg/frontend" "multi-organization fixture"

# 11. The schema stays parseable.
if [[ -f "$SCHEMA_FILE" ]] && ! python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$SCHEMA_FILE"; then
  fail "schemas/autospec-initiative.schema.json is not valid JSON"
fi

if [[ "$FAILURES" -gt 0 ]]; then
  printf 'autospec-initiative-contract: %d failure(s)\n' "$FAILURES" >&2
  exit 1
fi

printf 'autospec-initiative-contract: ok\n'
