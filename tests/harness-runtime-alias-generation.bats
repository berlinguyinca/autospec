#!/usr/bin/env bats

ROOT="${BATS_TEST_DIRNAME}/.."
GENERATOR="$ROOT/scripts/gen-harness-runtime-aliases.sh"

@test "canonical harness table deterministically generates shell fish and docs" {
  run bash "$GENERATOR" --check
  [ "$status" -eq 0 ]

  for id in claude codex opencode; do
    grep -F "autospec-env session -- $id" "$ROOT/templates/generated/harness-runtime-aliases.sh"
    grep -F "autospec-env session -- $id" "$ROOT/templates/generated/harness-runtime-aliases.fish"
    grep -F "| \`$id\` |" "$ROOT/docs/generated/harness-runtime-aliases.md"
  done
}

@test "generator rejects duplicate harness ids" {
  table="$BATS_TEST_TMPDIR/duplicate.tsv"
  printf 'codex\tcodex\t--yolo\tCodex CLI\ncodex\tcodex-beta\t\tCodex Beta\n' > "$table"

  run bash "$GENERATOR" --source "$table" --stdout sh

  [ "$status" -eq 2 ]
  [[ "$output" == *"duplicate harness id: codex"* ]]
}

@test "rollover launcher enters the same broker session before tmux launch" {
  grep -F 'autospec-session claude --dangerously-skip-permissions "$@"' \
    "$ROOT/templates/generated/harness-runtime-aliases.sh"
  grep -F 'autospec-session codex --yolo $argv' \
    "$ROOT/templates/generated/harness-runtime-aliases.fish"
  grep -F 'autospec-env session -- "$harness"' "$ROOT/scripts/autospec-session"
  awk '/tmux new-session/ && $0 !~ /autospec-env session --/ { bad=1 } END { exit bad }' \
    "$ROOT/scripts/autospec-session"
}

@test "generated Bash aliases and rollover wrappers source together" {
  run bash --noprofile --norc -ic 'source "$1"; type claude; type codex; type opencode' \
    bash "$ROOT/templates/generated/harness-runtime-aliases.sh"

  [ "$status" -eq 0 ]
  [[ "$output" == *"claude is a function"* ]]
  [[ "$output" == *"codex is a function"* ]]
  [[ "$output" == *"opencode is a function"* ]]
}

@test "harness detection reads the canonical table without permission flags" {
  grep -F 'harness-runtime-aliases.tsv' "$ROOT/scripts/lib/autospec-harness-detect.sh"
  ! grep -E -- '--yolo|--dangerously-skip-permissions' "$ROOT/scripts/lib/autospec-harness-detect.sh"
  run bash -c 'source "$1"; autospec_harness_supported_ids' bash \
    "$ROOT/scripts/lib/autospec-harness-detect.sh"
  [ "$status" -eq 0 ]
  [ "$output" = "$(cut -f1 "$ROOT/config/harness-runtime-aliases.tsv")" ]
}
