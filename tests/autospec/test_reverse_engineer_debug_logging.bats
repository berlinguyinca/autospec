#!/usr/bin/env bats

SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/reverse-engineer.sh"

@test "reverse-engineer orchestrator avoids debug logging APIs" {
  ! grep -Eq 'console\.(log|debug|info|warn|error)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)' "$SCRIPT"
}

@test "reverse-engineer inventory avoids debug logging APIs" {
  inventory="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/reverse-engineer/inventory.mjs"
  ! grep -Eq 'console\.(log|debug|info|warn|error)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)' "$inventory"
}

@test "reverse-engineer cluster avoids debug logging APIs" {
  cluster="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/reverse-engineer/cluster.mjs"
  ! grep -Eq 'console\.(log|debug|info|warn|error)|(^|[^[:alnum:]_])debugger([^[:alnum:]_]|$)' "$cluster"
}

@test "reverse-engineer cluster emits JSON through stdout" {
  cluster="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/reverse-engineer/cluster.mjs"
  run bash -c "printf '%s' '[]' | node '$cluster'"
  [ "$status" -eq 0 ]
  run jq -e '.significant == [] and .trivial == []' <<<"$output"
  [ "$status" -eq 0 ]
}

@test "reverse-engineer orchestrator remains shell-parseable" {
  run bash -n "$SCRIPT"
  [ "$status" -eq 0 ]
}

@test "reverse-engineer empty repository emits a manifest at runtime" {
  repo="$BATS_TEST_TMPDIR/repo"
  docs="$BATS_TEST_TMPDIR/docs"
  result="$BATS_TEST_TMPDIR/result.json"
  diagnostics="$BATS_TEST_TMPDIR/diagnostics.log"
  mkdir -p "$repo" "$docs"
  run bash -c 'bash "$1" --repo-root "$2" --docs-dir "$3" --date 2026-07-22 >"$4" 2>"$5"' \
    bash "$SCRIPT" "$repo" "$docs" "$result" "$diagnostics"
  [ "$status" -eq 0 ]
  jq -e '.written == [] and .skipped == [] and .manifest == []' "$result" >/dev/null
}
