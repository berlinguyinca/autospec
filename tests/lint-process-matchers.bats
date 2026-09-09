#!/usr/bin/env bats
# tests/lint-process-matchers.bats — issue #3938: reject command-line
# process matchers (pgrep/pkill with the -f flag) in repo scripts.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LINT="$REPO_ROOT/scripts/lint-process-matchers.sh"
FIX="$REPO_ROOT/tests/fixtures/lint-process-matchers"

@test "lint: --help exits 0 and mentions usage" {
  run bash "$LINT" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"Usage: scripts/lint-process-matchers.sh"* ]]
  [[ "$output" == *"autospec_wait_pid"* ]]
}

@test "lint: flags pgrep -f" {
  run bash "$LINT" "$FIX/bad-pgrep-f.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"PROCESS_MATCHER:"* ]]
  [[ "$output" == *":3:"* ]]
}

@test "lint: flags pkill -f" {
  run bash "$LINT" "$FIX/bad-pkill-f.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"PROCESS_MATCHER:"* ]]
  [[ "$output" == *":3:"* ]]
}

@test "lint: flags the self-matching wrapper case from #3938" {
  run bash "$LINT" "$FIX/wrapper-self-match.sh"
  [ "$status" -eq 1 ]
  [[ "$output" == *"PROCESS_MATCHER:"* ]]
  [[ "$output" == *"pgrep -f \"worker.sh\""* ]]
}

@test "lint: allows parent-PID matching (pgrep -P)" {
  run bash "$LINT" "$FIX/safe-pgrep-p.sh"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "lint: honors the allowlist file" {
  run bash "$LINT" "$FIX/allowlisted.sh"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "lint: honors a same-line waiver with a reason" {
  run bash "$LINT" "$FIX/waived-same-line.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"INFO:PROCESS_MATCHER:"* ]]
  [[ "$output" == *"waived: legacy integration reviewed in #3938"* ]]
}

@test "lint: honors a waiver on the line above" {
  run bash "$LINT" "$FIX/waived-line-above.sh"
  [ "$status" -eq 0 ]
  [[ "$output" == *"INFO:PROCESS_MATCHER:"* ]]
}

@test "lint: rejects a bare waiver without a reason" {
  local tmp
  tmp="$(mktemp)"
  printf '#!/usr/bin/env bash\n# process-matcher:allow\npgrep -f "x" >/dev/null 2>&1 || true\n' > "$tmp"
  run bash "$LINT" "$tmp"
  rm -f "$tmp"
  [ "$status" -eq 1 ]
  [[ "$output" == *"PROCESS_MATCHER:"* ]]
}

@test "lint: repo default scan is clean" {
  run bash "$LINT"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}
