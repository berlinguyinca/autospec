#!/usr/bin/env bats
# tests/autospec-log-state.bats — issue #3973: a single observation of an
# append-only source describes a moment, not a state. Exercised against
# the populated #3793 case: a status check against a log mid-write must
# report "running", not "stalled"; a freshly written but quiet log must
# NOT report running; destructive operations need a terminal marker or
# two independent observations; yield is a count, not a visible tail.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/.." && pwd)"
LIB="$REPO_ROOT/scripts/lib/autospec-log-state.sh"

setup() {
  DIR="$(mktemp -d)"
}

teardown() {
  jobs -p 2>/dev/null | xargs -r kill 2>/dev/null || true # linter:allow-VACUOUS_OR_TRUE cleanup, not an assertion — writer may already be dead
  rm -rf "$DIR"
}

# ── autospec_log_state: moment vs state ─────────────────────────────────────

@test "log_state: a log mid-write reports running, not stalled (populated #3793 case)" {
  {
    while :; do
      printf 'work line %s\n' "$(date +%s%N)" >> "$DIR/mid.log"
      sleep 0.2
    done
  } &
  local writer=$!
  sleep 0.3
  run bash -c ". '$LIB'; autospec_log_state '$DIR/mid.log' --gap 1"
  [ "$status" -eq 0 ]
  [ "$output" = "running" ]
  kill "$writer" 2>/dev/null
  wait "$writer" 2>/dev/null || true # linter:allow-VACUOUS_OR_TRUE reaps the writer; the assertion is above
}

@test "log_state: a terminal marker reports complete, without waiting" {
  printf 'header\n' > "$DIR/done.log"
  printf '######## complete ########\n' >> "$DIR/done.log"
  run bash -c ". '$LIB'; autospec_log_state '$DIR/done.log'"
  [ "$status" -eq 3 ]
  [ "$output" = "complete" ]
}

@test "log_state: a failure marker is terminal too" {
  printf 'header\n' > "$DIR/failed.log"
  printf '######## failed rc=1 ########\n' >> "$DIR/failed.log"
  run bash -c ". '$LIB'; autospec_log_state '$DIR/failed.log'"
  [ "$status" -eq 3 ]
  [ "$output" = "complete" ]
}

@test "log_state: a freshly written but quiet log reports stalled, not running" {
  # Written one moment ago, so recency is maximal — and still no evidence
  # of motion. Recency alone must never decide.
  printf 'only a header, no result yet\n' > "$DIR/quiet.log"
  run bash -c ". '$LIB'; autospec_log_state '$DIR/quiet.log' --gap 1"
  [ "$status" -eq 4 ]
  [ "$output" = "stalled" ]
}

@test "log_state: a missing log returns 1, usage returns 2" {
  run bash -c ". '$LIB'; autospec_log_state '$DIR/nope.log'"
  [ "$status" -eq 1 ]
  run bash -c ". '$LIB'; autospec_log_state"
  [ "$status" -eq 2 ]
  [[ "$output" == *usage* ]]
}

@test "log_state: an invalid marker regex returns 2" {
  printf 'x\n' > "$DIR/m.log"
  run bash -c ". '$LIB'; autospec_log_state '$DIR/m.log' --marker '[unclosed'"
  [ "$status" -eq 2 ]
}

# ── writer helpers: heartbeat and terminal marker ───────────────────────────

@test "run_step: heartbeats during the step, terminal marker after, rc propagated" {
  run bash -c ". '$LIB'; autospec_run_step '$DIR/step.log' --heartbeat-every 1 sleep 3"
  [ "$status" -eq 0 ]
  grep -q 'autospec-heartbeat' "$DIR/step.log"
  grep -qE '^######## complete ########$' "$DIR/step.log"
  # And the state reader agrees the step is done.
  run bash -c ". '$LIB'; autospec_log_state '$DIR/step.log'"
  [ "$status" -eq 3 ]
}

@test "run_step: a failing step leaves a failed marker and its exit code" {
  run bash -c ". '$LIB'; autospec_run_step '$DIR/fail.log' bash -c 'exit 7'"
  [ "$status" -eq 7 ]
  grep -q '^######## failed rc=7 ########$' "$DIR/fail.log"
}

@test "heartbeat and terminal: usage errors return 2" {
  run bash -c ". '$LIB'; autospec_heartbeat"
  [ "$status" -eq 2 ]
  [[ "$output" == *usage* ]]
  run bash -c ". '$LIB'; autospec_terminal"
  [ "$status" -eq 2 ]
  [[ "$output" == *usage* ]]
}

# ── autospec_destructive_guard: stop/re-dispatch/archive gate ────────────────

@test "destructive_guard: refuses while the step is running" {
  {
    while :; do
      printf 'still working %s\n' "$(date +%s%N)" >> "$DIR/busy.log"
      sleep 0.2
    done
  } &
  local writer=$!
  sleep 0.3
  run bash -c ". '$LIB'; autospec_destructive_guard '$DIR/busy.log' --gap 1"
  [ "$status" -eq 1 ]
  [[ "$output" == *REFUSING* ]]
  kill "$writer" 2>/dev/null
  wait "$writer" 2>/dev/null || true # linter:allow-VACUOUS_OR_TRUE reaps the writer; the assertion is above
}

@test "destructive_guard: allows on a terminal marker" {
  printf 'header\n' > "$DIR/done.log"
  printf '######## complete ########\n' >> "$DIR/done.log"
  run bash -c ". '$LIB'; autospec_destructive_guard '$DIR/done.log'"
  [ "$status" -eq 0 ]
  [[ "$output" == *ALLOWED* ]]
}

@test "destructive_guard: allows on the two-sample stall verdict" {
  printf 'header only\n' > "$DIR/still.log"
  run bash -c ". '$LIB'; autospec_destructive_guard '$DIR/still.log' --gap 1"
  [ "$status" -eq 0 ]
  [[ "$output" == *ALLOWED* ]]
  [[ "$output" == *"two samples"* ]]
}

@test "destructive_guard: fails closed when the log is missing" {
  run bash -c ". '$LIB'; autospec_destructive_guard '$DIR/absent.log'"
  [ "$status" -eq 1 ]
  [[ "$output" == *FAIL-CLOSED* ]]
}

# ── autospec_log_yield: a count, not a tail ──────────────────────────────────

@test "log_yield: counts the whole file, not the visible tail" {
  local i
  for i in 1 2 3 4 5; do
    printf 'opened PR: https://example.com/pull/%s\n' "$i" >> "$DIR/yield.log"
    # Filler after each count line, so the visible tail shows none.
    printf 'noise %s a\n' "$i" >> "$DIR/yield.log"
    printf 'noise %s b\n' "$i" >> "$DIR/yield.log"
  done
  [ "$(tail -n 2 "$DIR/yield.log" | grep -c 'PR: https' || true)" = "0" ]
  run bash -c ". '$LIB'; autospec_log_yield '$DIR/yield.log' 'PR: https'"
  [ "$status" -eq 0 ]
  [ "$output" = "5" ]
}

@test "log_yield: zero is a valid count; unreadable is an error" {
  printf 'nothing here\n' > "$DIR/empty.log"
  run bash -c ". '$LIB'; autospec_log_yield '$DIR/empty.log' 'PR: https'"
  [ "$status" -eq 0 ]
  [ "$output" = "0" ]
  run bash -c ". '$LIB'; autospec_log_yield '$DIR/missing.log' 'x'"
  [ "$status" -eq 1 ]
  run bash -c ". '$LIB'; autospec_log_yield '$DIR/empty.log'"
  [ "$status" -eq 2 ]
}

# ── the lib is syntactically clean ───────────────────────────────────────────

@test "lib: bash -n accepts it" {
  run bash -n "$LIB"
  [ "$status" -eq 0 ]
}
