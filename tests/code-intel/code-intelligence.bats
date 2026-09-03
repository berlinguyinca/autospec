#!/usr/bin/env bats
# tests/code-intel/code-intelligence.bats
#
# Validation coverage for the Code Intelligence Gateway: the checked-in operator
# config parses, the documented surface is actually documented, the upstream
# adapter stays confined to one table, and `autospec doctor code-intel` reports.
#
# Run: bats tests/code-intel/code-intelligence.bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
CONFIG=".autospec/code-intelligence.yaml"
DOC="docs/code-intelligence.md"
MODULE="crates/autospec-core/src/code_intel"
ADAPTER="$MODULE/backend/agent_lsp.rs"

setup() {
  cd "$REPO_ROOT" || exit 1
}

autospec_binary() {
  if [ -x "$REPO_ROOT/target/debug/autospec" ]; then
    echo "$REPO_ROOT/target/debug/autospec"
  elif [ -x "$REPO_ROOT/target/release/autospec" ]; then
    echo "$REPO_ROOT/target/release/autospec"
  else
    echo ""
  fi
}

@test "operator config, docs, and gateway module are checked in" {
  [ -f "$CONFIG" ]
  [ -f "$DOC" ]
  [ -d "$MODULE" ]
}

@test "operator config declares the supported schema version" {
  grep -qE '^version: 1$' "$CONFIG"
}

@test "operator config keeps every mandatory workflow gate enabled" {
  grep -qE '^  require_pre_change_impact: true$' "$CONFIG"
  grep -qE '^  require_post_change_diagnostics: true$' "$CONFIG"
  grep -qE '^  reviewer_independent_analysis: true$' "$CONFIG"
  grep -qE '^  block_new_errors: true$' "$CONFIG"
}

@test "operator config keeps the fail-closed security posture" {
  grep -qE '^  allow_public_bind: false$' "$CONFIG"
  grep -qE '^  trust_project_build_scripts: false$' "$CONFIG"
}

@test "operator config pins worktree isolation" {
  grep -qE '^  isolation: worktree$' "$CONFIG"
}

@test "every code.* operation in the schema is documented" {
  local operation
  for operation in find_symbol definition references implementations hover \
                   callers callees type_hierarchy diagnostics impact; do
    grep -q "code.${operation}" "$MODULE/schema.rs"
    grep -q "code.${operation}" "$DOC"
  done
}

@test "upstream agent-lsp tool names are confined to the adapter table" {
  local name
  for name in blast_radius goto_definition incoming_calls outgoing_calls; do
    grep -q "\"${name}\"" "$ADAPTER"
    # Any other module referencing an upstream tool name would couple AutoSpec
    # to agent-lsp's protocol; the adapter is the only permitted mention.
    run bash -c "grep -rl '\"${name}\"' '$MODULE' | grep -v 'backend/agent_lsp.rs'"
    [ -z "$output" ]
  done
}

@test "the adapter records a pinned upstream version" {
  grep -qE 'pub const PINNED_VERSION: &str = "[0-9]+\.[0-9]+\.[0-9]+";' "$ADAPTER"
}

@test "gateway sources stay under the 500-line complexity cap" {
  local file
  while IFS= read -r file; do
    local lines
    lines="$(wc -l < "$file")"
    [ "$lines" -le 500 ] || {
      echo "$file has $lines lines (cap 500)"
      return 1
    }
  done < <(find "$MODULE" -name '*.rs')
}

@test "doctor code-intel is documented in the CLI reference" {
  grep -q 'autospec doctor code-intel' docs/cli-reference.md
}

@test "doctor code-intel prints a JSON report" {
  local binary
  binary="$(autospec_binary)"
  [ -n "$binary" ] || skip "autospec binary not built"

  run "$binary" doctor code-intel --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"command":"doctor code-intel"'* ]]
  [[ "$output" == *'"backend":"agent-lsp"'* ]]
  [[ "$output" == *'"mode":"local"'* ]]
}

@test "doctor code-intel detects this repository's languages" {
  local binary
  binary="$(autospec_binary)"
  [ -n "$binary" ] || skip "autospec binary not built"

  run "$binary" doctor code-intel --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"language":"rust"'* ]]
}

@test "doctor code-intel prints a human-readable report" {
  local binary
  binary="$(autospec_binary)"
  [ -n "$binary" ] || skip "autospec binary not built"

  run "$binary" doctor code-intel
  [ "$status" -eq 0 ]
  [[ "$output" == *"AutoSpec code intelligence:"* ]]
  [[ "$output" == *"[ok] security"* ]]
}

@test "a malformed operator config fails loudly instead of defaulting" {
  local binary sandbox
  binary="$(autospec_binary)"
  [ -n "$binary" ] || skip "autospec binary not built"
  sandbox="$(mktemp -d)"
  mkdir -p "$sandbox/.autospec"
  printf 'version: 1\nworkflow:\n  block_new_error: false\n' \
    > "$sandbox/.autospec/code-intelligence.yaml"

  run bash -c "cd '$sandbox' && '$binary' doctor code-intel --json"
  rm -rf "$sandbox"
  [ "$status" -ne 0 ]
  [[ "$output" == *"unknown key in workflow"* ]]
}

# The gateway's own unit tests are owned by `cargo test --workspace`, not by this
# suite: shelling back into cargo from a validate check would rebuild the
# workspace inside a run cargo already launched.
@test "gateway unit tests are reachable from the workspace suite" {
  grep -q '#\[cfg(test)\]' "$MODULE/workspace.rs"
  grep -q '#\[cfg(test)\]' "$MODULE/gate.rs"
  grep -q '#\[cfg(test)\]' "$MODULE/diagnostics.rs"
  grep -q 'pub mod code_intel;' crates/autospec-core/src/lib.rs
}
