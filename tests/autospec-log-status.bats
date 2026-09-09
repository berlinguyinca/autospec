#!/usr/bin/env bats
# tests/autospec-log-status.bats — issue #3973: a system in motion must
# not be read as a system at rest. A status check against a mid-write log
# must report "running", not "stalled" (the populated #3793 case, where a
# guard and a reader both sampled a live process once and acted on the
# sample as if it were the state); "stalled" is only ever produced by two
# independent observations; destructive actions gate on a terminal marker
# or a double-observed verdict, never on one glance; and a yield
# characterisation is a computed count, not an estimate from the tail.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LIB="$REPO_ROOT/scripts/lib/autospec-log-status.sh"

# ── autospec_log_status: verdicts from markers and two samples ─────────────

@test "status: a terminal marker is final — one observation suffices" {
  local dir log
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  printf 'header\n' > "$log"
  run bash -c ". '$LIB'; autospec_log_terminal '$log'; autospec_log_status '$log' 2>/dev/null"
  [ "$status" -eq 0 ]
  [ "$output" = "complete" ]
  rm -rf "$dir"
}

@test "status: a mid-write log reports running, never stalled (populated #3793 case)" {
  # A pass is between writes when we look: the result line is "not yet",
  # not "not ever". The writer appends a line every 0.2s for ~10s; the
  # status check samples with a 1s gap and must see the log advance.
  local dir log writer
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  printf '######## start ########\n' > "$log"
  bash -c 'for i in $(seq 1 50); do printf "work line %s\n" "$i" >> "$1"; sleep 0.2; done' _ "$log" &
  writer=$!
  sleep 0.3
  run bash -c ". '$LIB'; autospec_log_status '$log' '' 1 2>/dev/null"
  [ "$status" -eq 0 ]
  [ "$output" = "running" ]
  kill "$writer" 2>/dev/null || true
  wait "$writer" 2>/dev/null || true
  rm -rf "$dir"
}

@test "status: a marker written within the gap is read as complete, not stalled" {
  local dir log writer
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  printf 'work\n' > "$log"
  # The pass finishes 0.5s into the 1s gap: silence at sample 1 must not
  # have been read as absence.
  bash -c 'sleep 0.5; . '"$LIB"'; autospec_log_terminal '"$log"'' &
  writer=$!
  sleep 0.2
  run bash -c ". '$LIB'; autospec_log_status '$log' '' 1 2>/dev/null"
  [ "$status" -eq 0 ]
  [ "$output" = "complete" ]
  wait "$writer" 2>/dev/null || true
  rm -rf "$dir"
}

@test "status: stalled requires two observations, and the report keeps them apart" {
  local dir log err
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  err="$dir/err"
  printf 'work\n' > "$log"
  # A quiet log: the only verdict two identical samples support is
  # "no progress over this interval".
  run bash -c ". '$LIB'; autospec_log_status '$log' '' 1 2>'$err'"
  [ "$status" -eq 0 ]
  # stdout carries only the verdict; the observation evidence (both
  # samples, with sizes and digests) stays on stderr under `obs:` and the
  # inference is labelled as such.
  [ "$output" = "stalled" ]
  grep -q 'sample 1' "$err"
  grep -q 'sample 2' "$err"
  grep -q 'inference: stalled' "$err"
  rm -rf "$dir"
}

@test "status: an unreadable log is an error, not a verdict" {
  local err
  err="$(mktemp)"
  run bash -c ". '$LIB'; autospec_log_status /nonexistent-dir-3973/pass.log 2>'$err'"
  [ "$status" -eq 1 ]
  # An error is reported on stderr, and no verdict is printed to stdout.
  [ -z "$output" ]
  grep -q 'cannot read log' "$err"
  rm -f "$err"
}

@test "status: usage errors return 2 (no logfile; non-numeric gap)" {
  run bash -c ". '$LIB'; autospec_log_status"
  [ "$status" -eq 2 ]
  [[ "$output" == *usage* ]]
  local dir log
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  : > "$log"
  run bash -c ". '$LIB'; autospec_log_status '$log' '' never"
  [ "$status" -eq 2 ]
  rm -rf "$dir"
}

# ── autospec_log_gate: destructive actions need a marker or two samples ────

@test "gate: refuses a destructive action while the log is running" {
  local dir log writer
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  printf '######## start ########\n' > "$log"
  bash -c 'for i in $(seq 1 50); do printf "work line %s\n" "$i" >> "$1"; sleep 0.2; done' _ "$log" &
  writer=$!
  sleep 0.3
  run bash -c ". '$LIB'; autospec_log_gate '$log' '' 1"
  [ "$status" -eq 1 ]
  [[ "$output" == *REFUSING* ]]
  kill "$writer" 2>/dev/null || true
  wait "$writer" 2>/dev/null || true
  rm -rf "$dir"
}

@test "gate: allows on complete (terminal marker) and on double-observed stalled" {
  local dir log
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  printf 'work\n' > "$log"
  run bash -c ". '$LIB'; autospec_log_gate '$log' '' 1"
  [ "$status" -eq 0 ]
  run bash -c ". '$LIB'; autospec_log_terminal '$log'; autospec_log_gate '$log'"
  [ "$status" -eq 0 ]
  rm -rf "$dir"
}

@test "gate: usage error without a logfile returns 2" {
  run bash -c ". '$LIB'; autospec_log_gate"
  [ "$status" -eq 2 ]
  [[ "$output" == *usage* ]]
}

# ── autospec_log_count: a number is computed, not estimated from the tail ──

@test "count: counts the whole log — the 86 PRs behind two visible hold lines" {
  # The destructive case from the issue: a characterisation of "low yield"
  # drawn from the last two log lines, when the same log held 86 results.
  local dir log i
  dir="$(mktemp -d)"
  log="$dir/convert.log"
  for i in $(seq 1 86); do
    printf 'PR: https://github.com/example/repo/pull/%s\n' "$i"
  done > "$log"
  printf 'hold: suite x\nhold: suite y\n' >> "$log"
  run bash -c ". '$LIB'; autospec_log_count '$log' 'PR: https'"
  [ "$status" -eq 0 ]
  [ "$output" = "86" ]
  rm -rf "$dir"
}

@test "count: a computed zero is a result, not an error" {
  local dir log
  dir="$(mktemp -d)"
  log="$dir/convert.log"
  printf 'hold: suite x\n' > "$log"
  run bash -c ". '$LIB'; autospec_log_count '$log' 'PR: https'"
  [ "$status" -eq 0 ]
  [ "$output" = "0" ]
  run bash -c ". '$LIB'; autospec_log_count '$log'"
  [ "$status" -eq 2 ]
  rm -rf "$dir"
}

# ── emitters: every long-running step can make its state askable ──────────

@test "heartbeat: appends a timestamped line a reader can distinguish from a result" {
  local dir log
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  printf 'work\n' > "$log"
  run bash -c ". '$LIB'; autospec_log_heartbeat '$log' 'convert3'"
  [ "$status" -eq 0 ]
  [ "$(grep -c '^heartbeat: convert3 ' "$log")" -eq 1 ]
  grep -qE '^heartbeat: convert3 [0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$' "$log"
  rm -rf "$dir"
}

@test "terminal: the default marker is the one status looks for" {
  local dir log
  dir="$(mktemp -d)"
  log="$dir/pass.log"
  : > "$log"
  bash -c ". '$LIB'; autospec_log_terminal '$log'"
  run bash -c ". '$LIB'; autospec_log_status '$log' 2>/dev/null"
  [ "$status" -eq 0 ]
  [ "$output" = "complete" ]
  rm -rf "$dir"
}

# ── the lib itself respects the -f matcher lint ────────────────────────────

@test "lib is clean under scripts/lint-process-matchers.sh" {
  run bash "$REPO_ROOT/scripts/lint-process-matchers.sh" "$LIB"
  [ "$status" -eq 0 ]
}
