#!/usr/bin/env bats

ROOT="${BATS_TEST_DIRNAME}/.."
GENERATOR="$ROOT/scripts/gen-harness-runtime-aliases.sh"

@test "canonical harness table deterministically generates shell fish and docs" {
  run bash "$GENERATOR" --check
  [ "$status" -eq 0 ]

  for id in claude codex opencode pi; do
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
  run bash --noprofile --norc -ic 'source "$1"; type claude; type codex; type opencode; type pi' \
    bash "$ROOT/templates/generated/harness-runtime-aliases.sh"

  [ "$status" -eq 0 ]
  [[ "$output" == *"claude is a function"* ]]
  [[ "$output" == *"codex is a function"* ]]
  [[ "$output" == *"opencode is a function"* ]]
  [[ "$output" == *"pi is a function"* ]]
}

@test "harness detection reads the canonical table without permission flags" {
  grep -F 'harness-runtime-aliases.tsv' "$ROOT/scripts/lib/autospec-harness-detect.sh"
  ! grep -E -- '--yolo|--dangerously-skip-permissions' "$ROOT/scripts/lib/autospec-harness-detect.sh"
  run env AUTOSPEC_HARNESS_RUNTIME_ALIASES="$ROOT/config/harness-runtime-aliases.tsv" \
    bash -c 'source "$1"; autospec_harness_supported_ids' bash \
    "$ROOT/scripts/lib/autospec-harness-detect.sh"
  [ "$status" -eq 0 ]
  [ "$output" = "$(cut -f1 "$ROOT/config/harness-runtime-aliases.tsv")" ]
}

@test "Pi is detected and receives the canonical autospec handoff" {
  stub_dir="$BATS_TEST_TMPDIR/pi-bin"
  mkdir -p "$stub_dir"
  cat > "$stub_dir/pi" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$PI_ARGS_FILE"
SH
  chmod +x "$stub_dir/pi"
  args_file="$BATS_TEST_TMPDIR/pi-args"

  run env PATH="$stub_dir:$PATH" \
    AUTOSPEC_HARNESS_RUNTIME_ALIASES="$ROOT/config/harness-runtime-aliases.tsv" \
    AUTOSPEC_HANDOFF_DISPATCHER=1 AUTOSPEC_HANDOFF_DISPATCHER_KIND=pi \
    PI_ARGS_FILE="$args_file" bash -c '
      source "$1"
      autospec_harness_invoke autonomous "route this task"
    ' bash "$ROOT/scripts/lib/autospec-harness-detect.sh"

  [ "$status" -eq 0 ]
  grep -Fx -- '--mode' "$args_file"
  grep -Fx -- 'json' "$args_file"
  grep -Fx -- '--print' "$args_file"
  grep -Fx -- '/autospec --autonomous route this task' "$args_file"
}
