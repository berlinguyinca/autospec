#!/usr/bin/env bats

ROOT="${BATS_TEST_DIRNAME}/.."
ADAPTER="$ROOT/scripts/autospec-runtime-worktree-cleanup.sh"

setup() {
  TEST_TMP="$(mktemp -d)"
  mkdir -p "$TEST_TMP/worktree"
  export AUTOSPEC_BIN="$ROOT/target/debug/autospec"
  export AGENT_ENV_STATE_ROOT="$TEST_TMP/state"
  [ -x "$AUTOSPEC_BIN" ] || cargo build -q -p autospec-cli --manifest-path "$ROOT/Cargo.toml"
}

teardown() { rm -rf "$TEST_TMP"; }

@test "cleanup adapter delegates to the real runtime collector" {
  run bash "$ADAPTER" "$TEST_TMP/worktree"
  [ "$status" -eq 0 ]
  [[ "$output" == *"AUTOSPEC_RUNTIME_GC_REMOVED=0"* ]]
  grep -F 'exec "$runtime" runtime env gc --repo "$1"' "$ADAPTER"
}

@test "cleanup adapter propagates broker failure" {
  mkdir -p "$AGENT_ENV_STATE_ROOT/broken-environment"
  printf '{}\n' > "$AGENT_ENV_STATE_ROOT/broken-environment/owner.json"
  printf '{}\n' > "$AGENT_ENV_STATE_ROOT/broken-environment/plan.json"
  printf '{}\n' > "$AGENT_ENV_STATE_ROOT/broken-environment/inventory.json"
  run bash "$ADAPTER" "$TEST_TMP/worktree"
  [ "$status" -eq 2 ]
  [[ "$output" == *"could not parse runtime state"* ]]
}

@test "cleanup adapter rejects a missing worktree argument" {
  run bash "$ADAPTER"
  [ "$status" -eq 2 ]
}
